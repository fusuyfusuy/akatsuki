//! Model Context Protocol (MCP) native JSON-RPC 2.0 stdio server.

use std::io::{self, BufRead, Write};
use std::path::Path;
use anyhow::Result;
use serde_json::{json, Value};

use crate::constants::MCP_DEFAULT_LIMIT;
use crate::graph::{calculate_blast_radius, extract_contract, traverse_graph};
use crate::mutations::{append_work_log, write_note};
use crate::search::{format_hits_compact, search_vault};
use crate::storage::{extract_section, resolve_note_file};
use crate::verify::{lint_vault, run_verification_tests, verify_links};

pub fn run_mcp_server(vault: &Path) -> Result<()> {
    eprintln!("🌅 Akatsuki Native MCP Server listening on stdio (PID: {})...", std::process::id());

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line_res in stdin.lock().lines() {
        let line = match line_res {
            Ok(l) => l,
            Err(_) => break,
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let req: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                let err_resp = json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32700, "message": format!("Parse error: {}", e) }
                });
                writeln!(stdout, "{}", serde_json::to_string(&err_resp)?)?;
                stdout.flush()?;
                continue;
            }
        };

        let id = req.get("id").cloned();
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or_else(|| json!({}));

        let response = match method {
            "initialize" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "serverInfo": {
                        "name": "akatsuki",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "capabilities": {
                        "tools": {}
                    }
                }
            }),
            "tools/list" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "tools": get_tool_definitions()
                }
            }),
            "tools/call" => {
                let tool_name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let tool_args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));

                let (text_out, is_err) = dispatch_tool(vault, tool_name, &tool_args);

                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [
                            {
                                "type": "text",
                                "text": text_out
                            }
                        ],
                        "isError": is_err
                    }
                })
            }
            "ping" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {}
            }),
            "notifications/initialized" => continue,
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("Method '{}' not found", method) }
            }),
        };

        writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
        stdout.flush()?;
    }

    Ok(())
}

fn dispatch_tool(vault: &Path, name: &str, args: &Value) -> (String, bool) {
    match name {
        "akatsuki_search" => {
            let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let domain = args.get("domain").and_then(|v| v.as_str());
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(MCP_DEFAULT_LIMIT as u64) as usize;
            let with_graph = args.get("with_graph").and_then(|v| v.as_bool()).unwrap_or(false);

            // Default mode is hybrid if models are downloaded, else bm25
            let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or_else(|| {
                if crate::vectors::are_models_available() {
                    "hybrid"
                } else {
                    "bm25"
                }
            });

            match search_vault(vault, query, domain, limit, with_graph, mode) {
                Ok(hits) => (format_hits_compact(&hits), false),
                Err(e) => (format!("Search failed: {}", e), true),
            }
        }
        "akatsuki_read" => {
            // Parameter aliasing: accept note, path, or target
            let target_query = args.get("note")
                .or_else(|| args.get("path"))
                .or_else(|| args.get("target"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if target_query.is_empty() {
                return ("Error: Missing required parameter 'note' (or 'path').".to_string(), true);
            }

            let section = args.get("section").and_then(|v| v.as_str());

            match resolve_note_file(vault, target_query) {
                Some(p) => match std::fs::read_to_string(&p) {
                    Ok(content) => {
                        if let Some(sec) = section {
                            match extract_section(&content, sec) {
                                Some(s) => (s, false),
                                None => (format!("Section '{}' not found in '{}'", sec, p.display()), true),
                            }
                        } else {
                            (content, false)
                        }
                    }
                    Err(e) => (format!("Failed to read note '{}': {}", p.display(), e), true),
                },
                None => (format!("Note '{}' not found in vault.", target_query), true),
            }
        }
        "akatsuki_contract" => {
            let note = args.get("note").and_then(|v| v.as_str()).unwrap_or("");
            if note.is_empty() {
                return ("Error: Missing required parameter 'note'.".to_string(), true);
            }
            match extract_contract(vault, note) {
                Ok(contract) => (serde_json::to_string_pretty(&contract).unwrap_or_default(), false),
                Err(e) => (format!("Contract extraction failed: {}", e), true),
            }
        }
        "akatsuki_blast" => {
            let target = args.get("target").and_then(|v| v.as_str()).unwrap_or("");
            if target.is_empty() {
                return ("Error: Missing required parameter 'target'.".to_string(), true);
            }
            match calculate_blast_radius(vault, target) {
                Ok(blast) => (serde_json::to_string_pretty(&blast).unwrap_or_default(), false),
                Err(e) => (format!("Blast radius failed: {}", e), true),
            }
        }
        "akatsuki_map" => {
            let target = args.get("target").and_then(|v| v.as_str()).unwrap_or("");
            let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(2) as usize;
            let direction = args.get("direction").and_then(|v| v.as_str()).unwrap_or("both");

            match traverse_graph(vault, target, depth, direction) {
                Ok(graph) => (serde_json::to_string_pretty(&graph).unwrap_or_default(), false),
                Err(e) => (format!("Map traversal failed: {}", e), true),
            }
        }
        "akatsuki_services" => {
            match crate::index::open_cache_db(vault) {
                Ok(con) => {
                    let sql = "SELECT name, container_prefix, ports, replicas, role, host, network FROM services";
                    match con.prepare(sql) {
                        Ok(mut stmt) => {
                            let rows = stmt.query_map([], |r| {
                                Ok(json!({
                                    "name": r.get::<_, String>(0)?,
                                    "container_prefix": r.get::<_, String>(1)?,
                                    "ports": r.get::<_, String>(2)?,
                                    "replicas": r.get::<_, String>(3)?,
                                    "role": r.get::<_, String>(4)?,
                                    "host": r.get::<_, String>(5)?,
                                    "network": r.get::<_, String>(6)?,
                                }))
                            });
                            match rows {
                                Ok(iter) => {
                                    let items: Vec<Value> = iter.filter_map(Result::ok).collect();
                                    (serde_json::to_string_pretty(&items).unwrap_or_default(), false)
                                }
                                Err(e) => (format!("Query failed: {}", e), true),
                            }
                        }
                        Err(e) => (format!("Prepare failed: {}", e), true),
                    }
                }
                Err(e) => (format!("Database open failed: {}", e), true),
            }
        }
        "akatsuki_projects" => {
            match crate::index::open_cache_db(vault) {
                Ok(con) => {
                    let sql = "SELECT stem, title, status, repo, host, network, summary FROM entities WHERE domain = '20-Projects'";
                    match con.prepare(sql) {
                        Ok(mut stmt) => {
                            let rows = stmt.query_map([], |r| {
                                Ok(json!({
                                    "stem": r.get::<_, String>(0)?,
                                    "title": r.get::<_, String>(1)?,
                                    "status": r.get::<_, Option<String>>(2)?,
                                    "repo": r.get::<_, Option<String>>(3)?,
                                    "host": r.get::<_, Option<String>>(4)?,
                                    "network": r.get::<_, Option<String>>(5)?,
                                    "summary": r.get::<_, Option<String>>(6)?,
                                }))
                            });
                            match rows {
                                Ok(iter) => {
                                    let items: Vec<Value> = iter.filter_map(Result::ok).collect();
                                    (serde_json::to_string_pretty(&items).unwrap_or_default(), false)
                                }
                                Err(e) => (format!("Query failed: {}", e), true),
                            }
                        }
                        Err(e) => (format!("Prepare failed: {}", e), true),
                    }
                }
                Err(e) => (format!("Database open failed: {}", e), true),
            }
        }
        "akatsuki_record_log" => {
            let project = args.get("project").and_then(|v| v.as_str()).unwrap_or("");
            let summary = args.get("summary").and_then(|v| v.as_str()).unwrap_or("");
            let device = args.get("device").and_then(|v| v.as_str());

            if summary.is_empty() {
                return ("Error: Missing parameter 'summary'.".to_string(), true);
            }

            match append_work_log(vault, project, summary, device) {
                Ok(entry) => (format!("Recorded: {}", entry), false),
                Err(e) => (format!("Log append failed: {}", e), true),
            }
        }
        "akatsuki_write_note" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let overwrite = args.get("overwrite").and_then(|v| v.as_bool()).unwrap_or(false);
            let raw = args.get("raw").and_then(|v| v.as_bool()).unwrap_or(false);

            if path.is_empty() || content.is_empty() {
                return ("Error: Missing required parameter 'path' or 'content'.".to_string(), true);
            }

            match write_note(vault, path, content, overwrite, raw) {
                Ok(p) => (format!("Note successfully written to: {}", p.display()), false),
                Err(e) => (format!("Write note failed: {}", e), true),
            }
        }
        "akatsuki_verify" => match verify_links(vault) {
            Ok(rep) => (serde_json::to_string_pretty(&rep).unwrap_or_default(), !rep.passed),
            Err(e) => (format!("Verification failed: {}", e), true),
        },
        "akatsuki_lint" => match lint_vault(vault) {
            Ok(rep) => (serde_json::to_string_pretty(&rep).unwrap_or_default(), !rep.passed),
            Err(e) => (format!("Lint failed: {}", e), true),
        },
        "akatsuki_test" => {
            let note = args.get("note").or_else(|| args.get("target")).and_then(|v| v.as_str());
            let dry_run = args.get("dry_run").and_then(|v| v.as_bool()).unwrap_or(false);
            match run_verification_tests(vault, note, dry_run) {
                Ok(rep) => (serde_json::to_string_pretty(&rep).unwrap_or_default(), rep.failed > 0),
                Err(e) => (format!("Tests failed: {}", e), true),
            }
        }
        _ => (format!("Tool '{}' not implemented", name), true),
    }
}

fn get_tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "akatsuki_search",
            "description": "Fast hybrid/BM25 search across living architecture, systems catalog, and notes. Compact response (<=2.5 KB).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query or natural language question" },
                    "domain": { "type": "string", "description": "Optional domain (e.g. '40-Systems', '20-Projects')" },
                    "limit": { "type": "integer", "default": 5, "description": "Max results to return (default: 5)" },
                    "mode": { "type": "string", "enum": ["hybrid", "bm25", "vector"], "description": "Retrieval mode" },
                    "with_graph": { "type": "boolean", "default": false, "description": "Include upstream/downstream relations" }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "akatsuki_read",
            "description": "Read a note or section from Akatsuki. Accepts note title, path, or target interchangeably.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "note": { "type": "string", "description": "Note title, stem, or relative path (e.g. 'Dokploy API Guide', '40-Systems/Cluster-Topology.md')" },
                    "path": { "type": "string", "description": "Alias for note" },
                    "section": { "type": "string", "description": "Optional section heading to extract" }
                }
            }
        }),
        json!({
            "name": "akatsuki_contract",
            "description": "Extract machine boundary contract (ports, relations, invariants, verifications), eliminating 80% narrative tokens.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "note": { "type": "string", "description": "Target note title, stem, or relative path" }
                },
                "required": ["note"]
            }
        }),
        json!({
            "name": "akatsuki_blast",
            "description": "Calculate architectural blast radius (upstream dependents, downstream dependencies, boundary sinks).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": { "type": "string", "description": "Target service, stem, or note name" }
                },
                "required": ["target"]
            }
        }),
        json!({
            "name": "akatsuki_map",
            "description": "Traverse knowledge graph relations up to specified depth.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": { "type": "string", "description": "Origin note stem or service name" },
                    "depth": { "type": "integer", "default": 2, "description": "Traversal depth" },
                    "direction": { "type": "string", "enum": ["both", "down", "up"], "default": "both" }
                },
                "required": ["target"]
            }
        }),
        json!({
            "name": "akatsuki_services",
            "description": "List all live containerized services, container prefixes, port allocations, and network boundaries.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "akatsuki_projects",
            "description": "List all active software project architectures, repository paths, and deployment statuses.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "akatsuki_record_log",
            "description": "Append a telegraphic work log entry into today's daily note under kernel lock.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Project identifier (e.g. 'web', 'toku')" },
                    "summary": { "type": "string", "description": "Telegraphic caveman summary: <verb> <target> -> <delta>; exit <N>" },
                    "device": { "type": "string", "description": "Optional device identifier" }
                },
                "required": ["summary"]
            }
        }),
        json!({
            "name": "akatsuki_write_note",
            "description": "Create or replace a note in the vault under kernel lock with instant SQLite index synchronization.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Vault-relative path (e.g. '20-Projects/my-app.md')" },
                    "content": { "type": "string", "description": "Full Markdown content with YAML frontmatter" },
                    "overwrite": { "type": "boolean", "default": false }
                },
                "required": ["path", "content"]
            }
        }),
        json!({
            "name": "akatsuki_verify",
            "description": "Validate bidirectional wikilink closure and detect broken links or orphan notes across the vault.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "akatsuki_lint",
            "description": "Statically lint notes for strict YAML frontmatter and verify no repo unit test boundary violations.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "akatsuki_test",
            "description": "Run living machine invariant tests (ports, containers, daemons) embedded in bash:verify blocks.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "note": { "type": "string", "description": "Optional note filter" },
                    "dry_run": { "type": "boolean", "default": false }
                }
            }
        }),
    ]
}
