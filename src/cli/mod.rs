//! Command-line interface definitions and dispatcher using clap.

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::constants::{CLI_DEFAULT_LIMIT, VERSION};
use crate::graph::{calculate_blast_radius, extract_contract, traverse_graph};
use crate::index::open_synced_db;
use crate::mutations::{
    append_section_in_note, append_work_log, read_daily_note, replace_section_in_note,
    set_note_property, write_note,
};
use crate::search::{
    execute_sql_query, format_hits_compact, get_keypath, list_notes, search_vault,
};
use crate::storage::{extract_section, resolve_note_file, resolve_vault_path};
use crate::verify::{lint_vault, reconcile_vault, run_verification_tests, verify_links};

#[derive(Parser)]
#[command(name = "akatsuki", version = VERSION, about = "Universal Living System Memory & Architectural Contracts Gateway")]
pub struct Cli {
    #[arg(long, global = true, help = "Explicit vault directory path")]
    pub vault: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    #[command(about = "Search living architecture notes via BM25, Candle vectors, or hybrid RRF")]
    Search {
        query: String,
        #[arg(short, long)]
        domain: Option<String>,
        #[arg(short = 'n', long, default_value_t = CLI_DEFAULT_LIMIT)]
        limit: usize,
        #[arg(short, long, default_value = "hybrid", value_parser = ["hybrid", "bm25", "vector"])]
        mode: String,
        #[arg(short = 'g', long)]
        with_graph: bool,
        #[arg(long)]
        compact: bool,
        #[arg(long)]
        json: bool,
    },

    #[command(
        alias = "cat",
        about = "Read an akatsuki note or specific markdown section"
    )]
    Read {
        note: String,
        #[arg(short, long)]
        section: Option<String>,
        #[arg(
            short,
            long,
            help = "Maximum approximate tokens (~4 chars each) to return"
        )]
        budget: Option<usize>,
        #[arg(long)]
        json: bool,
    },

    #[command(about = "Extract token-dense machine boundary contract from note")]
    Contract {
        note: String,
        #[arg(long)]
        json: bool,
    },

    #[command(about = "Calculate architectural blast radius for a service or note")]
    Blast {
        target: String,
        #[arg(long)]
        json: bool,
    },

    #[command(about = "Traverse knowledge graph relations from an origin note")]
    Map {
        target: String,
        #[arg(short, long, default_value_t = 2)]
        depth: usize,
        #[arg(long, default_value = "both", value_parser = ["both", "down", "up"])]
        direction: String,
        #[arg(long)]
        json: bool,
    },

    #[command(about = "List live containerized services, ports, and network topologies")]
    Services {
        #[arg(long)]
        json: bool,
    },

    #[command(about = "List active software project architectures and repository paths")]
    Projects {
        #[arg(long)]
        json: bool,
    },

    #[command(
        about = "O(1) exact property getter across services, entities, and note frontmatter"
    )]
    Get {
        keypath: String,
        #[arg(long)]
        json: bool,
    },

    #[command(about = "Execute read-only SQL query against SQLite projection cache")]
    Query {
        sql: String,
        #[arg(long)]
        json: bool,
    },

    #[command(about = "Record a telegraphic caveman work log entry into today's daily note")]
    Log {
        #[arg(short, long)]
        project: Option<String>,
        #[arg(short, long)]
        summary: String,
        #[arg(short, long)]
        device: Option<String>,
        #[arg(long)]
        json: bool,
    },

    #[command(about = "Write or update a note under process advisory lock")]
    Write {
        path: String,
        #[arg(short, long, allow_hyphen_values = true)]
        content: String,
        #[arg(long)]
        overwrite: bool,
        #[arg(long)]
        raw: bool,
        #[arg(long)]
        json: bool,
    },

    #[command(about = "Surgically replace a markdown section under a heading")]
    Replace {
        note: String,
        #[arg(long)]
        heading: String,
        #[arg(long, allow_hyphen_values = true)]
        content: String,
        #[arg(long)]
        json: bool,
    },

    #[command(about = "Set frontmatter YAML property on note")]
    Set {
        note: String,
        #[arg(short, long)]
        key: String,
        #[arg(short, long)]
        value: String,
        #[arg(long)]
        json: bool,
    },

    #[command(about = "Atomically append markdown bullets or text under a specific heading")]
    Append {
        note: String,
        #[arg(short = 'H', long)]
        heading: String,
        #[arg(short, long, allow_hyphen_values = true)]
        content: String,
        #[arg(long)]
        json: bool,
    },

    #[command(about = "Read today's or specified daily horizon and work ledger")]
    Daily {
        #[arg(short, long)]
        date: Option<String>,
        #[arg(long)]
        json: bool,
    },

    #[command(
        alias = "ls",
        about = "List notes in vault with optional domain filtering"
    )]
    List {
        #[arg(short, long)]
        domain: Option<String>,
        #[arg(long)]
        json: bool,
    },

    #[command(about = "Statically lint notes for YAML frontmatter and schema conformance")]
    Lint {
        #[arg(long)]
        json: bool,
    },

    #[command(about = "Verify bidirectional wikilinks and orphan notes across vault")]
    Verify {
        #[arg(long)]
        json: bool,
    },

    #[command(about = "Run living machine invariant tests (ports, containers, daemons)")]
    Test {
        note: Option<String>,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },

    #[command(
        about = "Rebuild the projection and repair frontmatter quoting and domain MOC links"
    )]
    Reconcile {
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },

    #[command(
        about = "Provision Hugging Face Candle E5-Small model weights into ~/.cache/akatsuki"
    )]
    SetupModels {},

    #[command(about = "Start Model Context Protocol (MCP) JSON-RPC 2.0 stdio server")]
    Mcp {},
}

pub fn run_cli(cli: Cli) -> Result<()> {
    let vault = resolve_vault_path(cli.vault.as_deref());

    match cli.command {
        Commands::Search {
            query,
            domain,
            limit,
            mode,
            with_graph,
            compact,
            json,
        } => {
            let hits = search_vault(&vault, &query, domain.as_deref(), limit, with_graph, &mode)?;
            let degradation = if mode == "bm25" {
                None
            } else {
                crate::vectors::hybrid_note(&vault)
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&hits)?);
            } else if compact {
                println!("{}", format_hits_compact(&hits));
                if let Some(note) = degradation {
                    println!("⚠ {}", note);
                }
            } else {
                println!("Found {} matching note(s) [Mode: {}]:", hits.len(), mode);
                if let Some(note) = degradation {
                    println!("  ⚠ {}", note);
                }
                for h in hits {
                    println!(
                        "\n- **{}** (`{}`) [Score: {}]: {}",
                        h.title, h.rel_path, h.score, h.summary
                    );
                    if let Some(ref b) = h.breadcrumb {
                        println!("    Section: {}", b);
                    }
                    if !h.snippet.is_empty() {
                        println!("    Snippet: {}", h.snippet);
                    }
                }
            }
        }
        Commands::Read {
            note,
            section,
            budget,
            json,
        } => {
            let note_path = resolve_note_file(&vault, &note)
                .ok_or_else(|| anyhow::anyhow!("Note '{}' not found in vault", note))?;
            let raw_content = std::fs::read_to_string(&note_path)?;
            let content = if let Some(sec) = section {
                extract_section(&raw_content, &sec).ok_or_else(|| {
                    anyhow::anyhow!("Section '{}' not found in '{}'", sec, note_path.display())
                })?
            } else {
                raw_content
            };
            let content = crate::storage::apply_token_budget(&content, budget);
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "path": note_path.display().to_string(), "content": content })
                );
            } else {
                println!("{}", content);
            }
        }
        Commands::Contract { note, json } => {
            let contract = extract_contract(&vault, &note)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&contract)?);
            } else {
                println!("{}", serde_yaml::to_string(&contract)?);
            }
        }
        Commands::Blast { target, json } => {
            let blast = calculate_blast_radius(&vault, &target)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&blast)?);
            } else {
                println!("# 💥 Architectural Blast Radius: `{}`\n", blast.target);
                println!("## ⬆️ Upstream Dependents:");
                for u in &blast.upstream {
                    println!("- **`{}`** ({})", u.source_rel, u.relation_type);
                }
                println!("\n## ⬇️ Downstream Dependencies:");
                for d in &blast.downstream {
                    println!("- **`{}`** ({})", d.target_stem, d.relation_type);
                }
                println!("\n## 🎯 Boundary Sinks & Ports:");
                for s in &blast.boundary_sinks {
                    println!(
                        "- **{}** (`{}`): {} [host: {}, net: {}]",
                        s.name, s.container_prefix, s.ports, s.host, s.network
                    );
                }
            }
        }
        Commands::Map {
            target,
            depth,
            direction,
            json,
        } => {
            let map = traverse_graph(&vault, &target, depth, &direction)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&map.payload)?);
            } else {
                println!("{}", map.text);
            }
        }
        Commands::Services { json } => {
            let con = open_synced_db(&vault)?;
            let mut stmt = con.prepare(
                "SELECT name, container_prefix, ports, replicas, role, host, network FROM services",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(serde_json::json!({
                    "name": r.get::<_, String>(0)?,
                    "container_prefix": r.get::<_, String>(1)?,
                    "ports": r.get::<_, String>(2)?,
                    "replicas": r.get::<_, String>(3)?,
                    "role": r.get::<_, String>(4)?,
                    "host": r.get::<_, String>(5)?,
                    "network": r.get::<_, String>(6)?,
                }))
            })?;
            let items: Vec<serde_json::Value> = rows.filter_map(Result::ok).collect();
            if json {
                println!("{}", serde_json::to_string_pretty(&items)?);
            } else {
                for s in items {
                    println!(
                        "- **{}** (ports: {}): {}",
                        s["name"].as_str().unwrap_or(""),
                        s["ports"].as_str().unwrap_or(""),
                        s["role"].as_str().unwrap_or("")
                    );
                }
            }
        }
        Commands::Projects { json } => {
            let con = open_synced_db(&vault)?;
            let mut stmt = con.prepare("SELECT stem, title, status, repo, host, network, summary FROM entities WHERE domain = '20-Projects'")?;
            let rows = stmt.query_map([], |r| {
                Ok(serde_json::json!({
                    "stem": r.get::<_, String>(0)?,
                    "title": r.get::<_, String>(1)?,
                    "status": r.get::<_, Option<String>>(2)?,
                    "repo": r.get::<_, Option<String>>(3)?,
                    "host": r.get::<_, Option<String>>(4)?,
                    "network": r.get::<_, Option<String>>(5)?,
                    "summary": r.get::<_, Option<String>>(6)?,
                }))
            })?;
            let items: Vec<serde_json::Value> = rows.collect::<rusqlite::Result<Vec<_>>>()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&items)?);
            } else {
                for p in items {
                    println!(
                        "- **{}** (`{}`): [{}] {}",
                        p["title"].as_str().unwrap_or(""),
                        p["stem"].as_str().unwrap_or(""),
                        p["status"].as_str().unwrap_or("unknown"),
                        p["summary"].as_str().unwrap_or("")
                    );
                }
            }
        }
        Commands::Get { keypath, json } => {
            let val = get_keypath(&vault, &keypath)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "keypath": keypath,
                        "value": val
                    }))?
                );
            } else {
                match val {
                    serde_json::Value::String(s) => println!("{}", s),
                    other => println!("{}", serde_json::to_string_pretty(&other)?),
                }
            }
        }
        Commands::Query { sql, json } => {
            let results = execute_sql_query(&vault, &sql)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&results)?);
            } else {
                for r in results {
                    println!("{}", serde_json::to_string(&r)?);
                }
            }
        }
        Commands::Log {
            project,
            summary,
            device,
            json,
        } => {
            let p_str = project.unwrap_or_default();
            let entry = append_work_log(&vault, &p_str, &summary, device.as_deref())?;
            if json {
                println!("{}", serde_json::json!({ "entry": entry }));
            } else {
                println!("Recorded: {}", entry);
            }
        }
        Commands::Write {
            path,
            content,
            overwrite,
            raw,
            json,
        } => {
            let p = write_note(&vault, &path, &content, overwrite, raw)?;
            if json {
                println!("{}", serde_json::json!({ "path": p.display().to_string() }));
            } else {
                println!("Note successfully written: {}", p.display());
            }
        }
        Commands::Replace {
            note,
            heading,
            content,
            json,
        } => {
            replace_section_in_note(&vault, &note, &heading, &content)?;
            if json {
                println!("{}", serde_json::json!({ "status": "ok" }));
            } else {
                println!("Section '{}' replaced successfully in '{}'", heading, note);
            }
        }
        Commands::Set {
            note,
            key,
            value,
            json,
        } => {
            set_note_property(&vault, &note, &key, &value)?;
            if json {
                println!("{}", serde_json::json!({ "status": "ok" }));
            } else {
                println!("Property '{}' updated in '{}'", key, note);
            }
        }
        Commands::Append {
            note,
            heading,
            content,
            json,
        } => {
            append_section_in_note(&vault, &note, &heading, &content)?;
            if json {
                println!("{}", serde_json::json!({ "status": "ok" }));
            } else {
                println!("Content appended under '{}' in '{}'", heading, note);
            }
        }
        Commands::Daily { date, json } => {
            let (content, exists) = read_daily_note(&vault, date.as_deref())?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "content": content, "exists": exists })
                );
            } else {
                println!("{}", content);
            }
        }
        Commands::List { domain, json } => {
            let notes = list_notes(&vault, domain.as_deref())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&notes)?);
            } else {
                for n in notes {
                    let stem = n["stem"].as_str().unwrap_or("");
                    let title = n["title"].as_str().unwrap_or("");
                    let rel = n["rel_path"].as_str().unwrap_or("");
                    println!("- **{}** (`{}`): {}", title, rel, stem);
                }
            }
        }
        Commands::Lint { json } => {
            let rep = lint_vault(&vault)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else if rep.passed {
                println!("PASSED: All {} notes conform to schema.", rep.total_notes);
            } else {
                println!("FAILED: {} violation(s) found:", rep.errors.len());
                for e in rep.errors {
                    println!("  - {}", e);
                }
            }
            if !rep.passed {
                std::process::exit(1);
            }
        }
        Commands::Verify { json } => {
            let rep = verify_links(&vault)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else if rep.passed {
                println!("PASSED: Zero broken wikilinks, zero orphan notes.");
            } else {
                println!("FAILED: Verification issues found:");
                for (src, tgt) in rep.broken_wikilinks {
                    println!("  - Broken wikilink in '{}' -> [[{}]]", src, tgt);
                }
                for (src, tgt) in rep.broken_markdown_links {
                    println!("  - Broken markdown link in '{}' -> ({})", src, tgt);
                }
                for orph in rep.orphan_notes {
                    println!("  - Orphan note: {}", orph);
                }
            }
            if !rep.passed {
                std::process::exit(1);
            }
        }
        Commands::Test {
            note,
            dry_run,
            json,
        } => {
            let rep = run_verification_tests(&vault, note.as_deref(), dry_run)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                if dry_run {
                    println!(
                        "Dry-run: {} assertion(s) discovered, none executed",
                        rep.total
                    );
                } else {
                    println!(
                        "Ran {} verification assertion(s): {} PASSED, {} FAILED",
                        rep.total, rep.passed, rep.failed
                    );
                }
                for item in rep.results {
                    if !item.executed {
                        println!("○ [{}] not executed: `{}`", item.source, item.command);
                    } else if item.passed {
                        println!("✅ [{}] exit 0: `{}`", item.source, item.command);
                    } else {
                        println!(
                            "❌ [{}] exit {}: `{}`",
                            item.source, item.exit_code, item.command
                        );
                        if !item.stderr.is_empty() {
                            println!("    stderr: {}", item.stderr);
                        }
                    }
                }
            }
            if rep.failed > 0 {
                std::process::exit(1);
            }
        }
        Commands::Reconcile { dry_run, json } => {
            let report = reconcile_vault(&vault, dry_run)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                let prefix = if dry_run { "[DRY-RUN] " } else { "" };
                println!(
                    "{prefix}Vault reconciliation completed in {:.2}ms ({} notes total, {} added, {} updated, {} deleted).",
                    report.sync.duration_ms,
                    report.sync.total,
                    report.sync.added,
                    report.sync.updated,
                    report.sync.deleted
                );
                for action in &report.actions {
                    println!("  - {}", action);
                }
                for err in &report.sync.parse_errors {
                    println!("  - unindexable frontmatter: {}", err);
                }
                if let Some(note) = &report.sync.vectors {
                    println!("  - vectors: {}", note);
                }
            }
        }
        Commands::SetupModels {} => {
            let status = crate::vectors::setup_models()?;
            println!("{}", status);
        }
        Commands::Mcp {} => {
            crate::mcp::run_mcp_server(&vault)?;
        }
    }

    Ok(())
}
