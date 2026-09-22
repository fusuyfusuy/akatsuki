//! Indexing engine: Parallel Blake3 Merkle scanner, SQLite WAL setup, and FTS5 synchronization.

use anyhow::{Context, Result};
use rayon::prelude::*;
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::storage::parse_frontmatter;

/// Renders a YAML scalar or collection as the text stored in a relation/invariant row.
fn scalar_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.trim().to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn scalar_join(items: &[Value]) -> String {
    items
        .iter()
        .map(scalar_text)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Extracts the bare note stem from a wikilink target or declared relation target.
/// Strips surrounding `[[` and `]]`, anchors (`#...`), `.md` suffix, and directory paths (`dir/stem`).
pub fn extract_target_stem(raw: &str) -> String {
    let mut s = raw.trim();
    if let Some(inner) = s.strip_prefix("[[").and_then(|v| v.strip_suffix("]]")) {
        s = inner.trim();
    }
    let target = s.split('|').next().unwrap_or(s).trim();
    let without_anchor = target.split('#').next().unwrap_or(target).trim();
    let without_md = without_anchor.strip_suffix(".md").unwrap_or(without_anchor);
    let stem = without_md
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(without_md)
        .trim();
    stem.to_string()
}

/// Parses wikilinks from note body, extracting target note stems.
/// Escaped wikilinks (`\[[x]]`) are ignored as documentation.
pub fn parse_wikilinks(body: &str) -> Vec<String> {
    use std::sync::OnceLock;
    static RE: OnceLock<Regex> = OnceLock::new();
    let wikilink_re = RE.get_or_init(|| {
        Regex::new(r"\[\[([^\]\|]+)(?:\|[^\]]+)?\]\]").expect("valid wikilink regex")
    });
    let mut targets = Vec::new();
    for caps in wikilink_re.captures_iter(body) {
        let whole = match caps.get(0) {
            Some(w) => w,
            None => continue,
        };
        if whole.start() > 0 && body.as_bytes().get(whole.start() - 1) == Some(&b'\\') {
            continue;
        }
        let clean_stem = extract_target_stem(&caps[1]);
        if !clean_stem.is_empty() {
            targets.push(clean_stem);
        }
    }
    targets
}

#[derive(Debug, Default, serde::Serialize)]
pub struct SyncReport {
    pub added: usize,
    pub updated: usize,
    pub deleted: usize,
    pub unchanged: usize,
    pub total: usize,
    pub parse_errors: Vec<String>,
    pub vectors: Option<String>,
    pub duration_ms: f64,
}
pub fn open_cache_db(vault: &Path) -> Result<Connection> {
    let ak_dir = vault.join(".akatsuki");
    fs::create_dir_all(&ak_dir)?;
    let db_path = ak_dir.join("cache.db");

    let con = Connection::open(&db_path)
        .with_context(|| format!("Failed to open database at {}", db_path.display()))?;

    // Performance pragmas
    con.pragma_update(None, "journal_mode", "WAL")?;
    con.pragma_update(None, "synchronous", "NORMAL")?;
    con.pragma_update(None, "foreign_keys", "ON")?;
    con.pragma_update(None, "temp_store", "MEMORY")?;
    con.pragma_update(None, "cache_size", "-32000")?; // 32MB cache

    init_tables(&con)?;

    Ok(con)
}

/// Opens the projection and brings it up to date with the vault in one call.
///
/// Read paths MUST use this: every queryable table is derived from markdown, so a
/// projection that was never reconciled answers "nothing found" for notes that exist.
pub fn open_synced_db(vault: &Path) -> Result<Connection> {
    let mut con = open_cache_db(vault)?;
    sync_vault_index(vault, &mut con)?;
    Ok(con)
}

/// Bumped whenever the projection's shape changes; a mismatch rebuilds the cache.
const SCHEMA_VERSION: &str = "0.2.2";

fn init_tables(con: &Connection) -> Result<()> {
    con.execute_batch("CREATE TABLE IF NOT EXISTS schema_meta (key TEXT PRIMARY KEY, val TEXT);")?;

    let stored: Option<String> = con
        .query_row(
            "SELECT val FROM schema_meta WHERE key = 'version'",
            [],
            |r| r.get(0),
        )
        .optional()?;

    if stored.as_deref() != Some(SCHEMA_VERSION) {
        // The cache is a pure projection of the markdown vault, so a version bump
        // rebuilds it wholesale instead of migrating columns in place.
        con.execute_batch(
            r#"
            DROP TABLE IF EXISTS notes_fts;
            DROP TABLE IF EXISTS file_meta;
            DROP TABLE IF EXISTS entities;
            DROP TABLE IF EXISTS services;
            DROP TABLE IF EXISTS relations;
            DROP TABLE IF EXISTS invariants;
            DROP TABLE IF EXISTS verifications;
            DROP TABLE IF EXISTS note_vectors;
            "#,
        )?;
        con.execute(
            "INSERT OR REPLACE INTO schema_meta (key, val) VALUES ('version', ?1)",
            params![SCHEMA_VERSION],
        )?;
    }

    con.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS file_meta (
            rel_path TEXT PRIMARY KEY,
            blake3_hash TEXT NOT NULL,
            size INTEGER NOT NULL,
            indexed_at TEXT NOT NULL
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
            rel_path UNINDEXED,
            stem UNINDEXED,
            domain,
            title,
            tags,
            summary,
            body,
            tokenize='unicode61 remove_diacritics 2'
        );

        CREATE TABLE IF NOT EXISTS entities (
            rel_path TEXT PRIMARY KEY,
            stem TEXT NOT NULL,
            domain TEXT NOT NULL,
            title TEXT NOT NULL,
            type TEXT NOT NULL,
            summary TEXT,
            status TEXT,
            repo TEXT,
            host TEXT,
            network TEXT,
            tags TEXT,
            updated TEXT,
            updated_by TEXT,
            metadata_json TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS services (
            name TEXT,
            container_prefix TEXT,
            ports TEXT,
            host TEXT,
            network TEXT,
            replicas TEXT,
            role TEXT,
            rel_path TEXT
        );

        CREATE TABLE IF NOT EXISTS relations (
            source_rel TEXT,
            target_stem TEXT,
            relation_type TEXT
        );

        CREATE TABLE IF NOT EXISTS invariants (
            source_rel TEXT,
            invariant_text TEXT
        );

        CREATE TABLE IF NOT EXISTS verifications (
            source_rel TEXT,
            command TEXT
        );

        CREATE TABLE IF NOT EXISTS note_vectors (
            chunk_id TEXT PRIMARY KEY,
            rel_path TEXT NOT NULL,
            stem TEXT NOT NULL,
            domain TEXT NOT NULL,
            display_title TEXT NOT NULL,
            display_summary TEXT NOT NULL,
            tags TEXT,
            breadcrumb TEXT,
            chunk_index INTEGER NOT NULL,
            total_chunks INTEGER NOT NULL,
            preview TEXT NOT NULL,
            vector_blob BLOB NOT NULL,
            dim INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_entities_stem ON entities(stem);
        CREATE INDEX IF NOT EXISTS idx_entities_title ON entities(title);
        CREATE INDEX IF NOT EXISTS idx_entities_type ON entities(type);
        CREATE INDEX IF NOT EXISTS idx_invariants_source ON invariants(source_rel);
        CREATE INDEX IF NOT EXISTS idx_verifications_source ON verifications(source_rel);
        CREATE INDEX IF NOT EXISTS idx_services_name ON services(name);
        CREATE INDEX IF NOT EXISTS idx_services_rel ON services(rel_path);
        CREATE INDEX IF NOT EXISTS idx_note_vectors_rel ON note_vectors(rel_path);
        CREATE INDEX IF NOT EXISTS idx_note_vectors_domain ON note_vectors(domain);
        CREATE INDEX IF NOT EXISTS idx_note_vectors_stem ON note_vectors(stem);
        CREATE INDEX IF NOT EXISTS idx_relations_source ON relations(source_rel);
        CREATE INDEX IF NOT EXISTS idx_relations_target ON relations(target_stem);
        "#,
    )?;

    Ok(())
}

struct ScannedFile {
    rel_path: String,
    hash: String,
    size: usize,
    content: String,
}

pub fn sync_vault_index(vault: &Path, con: &mut Connection) -> Result<SyncReport> {
    sync_vault_index_with(vault, con, true)
}

/// Reports what a reconcile would change without touching the projection.
pub fn sync_vault_index_dry_run(vault: &Path, con: &mut Connection) -> Result<SyncReport> {
    sync_vault_index_with(vault, con, false)
}

fn sync_vault_index_with(vault: &Path, con: &mut Connection, apply: bool) -> Result<SyncReport> {
    let t0 = std::time::Instant::now();

    // 1. Gather all existing indexed hashes
    let mut indexed_hashes: HashMap<String, String> = HashMap::new();
    {
        let mut stmt = con.prepare("SELECT rel_path, blake3_hash FROM file_meta")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for r in rows.flatten() {
            indexed_hashes.insert(r.0, r.1);
        }
    }

    // 2. Discover markdown files
    let ignored_dirs = [
        ".git",
        ".akatsuki",
        ".venv",
        "node_modules",
        ".obsidian",
        "__pycache__",
        "_templates",
    ];
    let walker = walkdir::WalkDir::new(vault).into_iter().filter_entry(|e| {
        let name = e.file_name().to_string_lossy();
        !ignored_dirs.iter().any(|ig| *ig == name)
    });

    let paths: Vec<PathBuf> = walker
        .flatten()
        .filter(|e| e.file_type().is_file() && e.path().extension().is_some_and(|ext| ext == "md"))
        .map(|e| e.into_path())
        .collect();

    // 3. Parallel Blake3 Merkle Scan via Rayon
    let scanned_files: Vec<ScannedFile> = paths
        .par_iter()
        .filter_map(|abs| {
            let rel = abs.strip_prefix(vault).ok()?.to_string_lossy().to_string();
            let content = fs::read_to_string(abs).ok()?;
            let hash = blake3::hash(content.as_bytes()).to_hex().to_string();
            let size = content.len();

            Some(ScannedFile {
                rel_path: rel,
                hash,
                size,
                content,
            })
        })
        .collect();

    let mut current_map: HashMap<String, ScannedFile> = HashMap::new();
    for f in scanned_files {
        current_map.insert(f.rel_path.clone(), f);
    }

    // 4. Calculate Diffs
    let mut to_delete = Vec::new();
    for rel in indexed_hashes.keys() {
        if !current_map.contains_key(rel) {
            to_delete.push(rel.clone());
        }
    }

    let mut to_update = Vec::new();
    let mut unchanged_count = 0;

    for (rel, file) in current_map {
        match indexed_hashes.get(&rel) {
            Some(existing_hash) if existing_hash == &file.hash => {
                unchanged_count += 1;
            }
            _ => {
                to_update.push(file);
            }
        }
    }

    let deleted_count = to_delete.len();
    let updated_count = to_update.len();
    let added_count = to_update
        .iter()
        .filter(|f| !indexed_hashes.contains_key(&f.rel_path))
        .count();
    let total_count = unchanged_count + updated_count;
    let mut parse_errors: Vec<String> = Vec::new();

    if !apply {
        let mut parse_errors = Vec::new();
        for file in &to_update {
            if let Err(e) = parse_frontmatter(&file.content) {
                parse_errors.push(format!("'{}': {}", file.rel_path, e));
            }
        }
        return Ok(SyncReport {
            added: added_count,
            updated: updated_count,
            deleted: deleted_count,
            unchanged: unchanged_count,
            total: total_count,
            parse_errors,
            vectors: None,
            duration_ms: t0.elapsed().as_secs_f64() * 1000.0,
        });
    }
    if to_delete.is_empty() && to_update.is_empty() {
        return Ok(SyncReport {
            added: 0,
            updated: 0,
            deleted: 0,
            unchanged: unchanged_count,
            total: total_count,
            parse_errors: Vec::new(),
            vectors: None,
            duration_ms: t0.elapsed().as_secs_f64() * 1000.0,
        });
    }

    // 5. Apply SQLite Transactions
    let tx = con.transaction()?;

    for del_rel in to_delete {
        tx.execute(
            "DELETE FROM file_meta WHERE rel_path = ?1",
            params![del_rel],
        )?;
        tx.execute(
            "DELETE FROM notes_fts WHERE rel_path = ?1",
            params![del_rel],
        )?;
        tx.execute("DELETE FROM entities WHERE rel_path = ?1", params![del_rel])?;
        tx.execute("DELETE FROM services WHERE rel_path = ?1", params![del_rel])?;
        tx.execute(
            "DELETE FROM relations WHERE source_rel = ?1",
            params![del_rel],
        )?;
        tx.execute(
            "DELETE FROM invariants WHERE source_rel = ?1",
            params![del_rel],
        )?;
        tx.execute(
            "DELETE FROM verifications WHERE source_rel = ?1",
            params![del_rel],
        )?;
        tx.execute(
            "DELETE FROM note_vectors WHERE rel_path = ?1",
            params![del_rel],
        )?;
    }

    let verify_re = Regex::new(r"(?s)```bash:verify\s*\n(.*?)\n```")?;

    let now_iso = chrono::Utc::now().to_rfc3339();

    // Notes whose text changed are the only ones that need re-embedding.
    let mut vector_sources: Vec<(String, String)> = Vec::new();

    for file in &to_update {
        let rel = &file.rel_path;
        let content = &file.content;
        let stem = Path::new(rel)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(rel);
        let domain = rel.split('/').next().unwrap_or("").to_string();
        if crate::vectors::feature_enabled() {
            vector_sources.push((rel.clone(), content.clone()));
        }

        let parsed = parse_frontmatter(content);
        let (fm, body) = match parsed {
            Ok(parts) => parts,
            Err(e) => {
                parse_errors.push(format!("'{}': {}", rel, e));
                (serde_json::json!({}), content.clone())
            }
        };

        let title = fm
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or(stem)
            .to_string();
        let summary = fm
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let status = fm
            .get("status")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let repo = fm
            .get("repo")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let host = fm
            .get("host")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let network = fm
            .get("network")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let tags_str = match fm.get("tags") {
            Some(Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            Some(Value::String(s)) => s.clone(),
            _ => String::new(),
        };
        let note_type = fm
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("note")
            .to_string();
        let updated = fm
            .get("updated")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let updated_by = fm
            .get("updated_by")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let metadata_json = serde_json::to_string(&fm).unwrap_or_else(|_| "{}".to_string());

        // Clean prior records
        tx.execute("DELETE FROM notes_fts WHERE rel_path = ?1", params![rel])?;
        tx.execute("DELETE FROM entities WHERE rel_path = ?1", params![rel])?;
        tx.execute("DELETE FROM services WHERE rel_path = ?1", params![rel])?;
        tx.execute("DELETE FROM relations WHERE source_rel = ?1", params![rel])?;
        tx.execute("DELETE FROM invariants WHERE source_rel = ?1", params![rel])?;
        tx.execute(
            "DELETE FROM verifications WHERE source_rel = ?1",
            params![rel],
        )?;

        // Insert notes_fts
        tx.execute(
            "INSERT INTO notes_fts (rel_path, stem, domain, title, tags, summary, body) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![rel, stem, domain, title, tags_str, summary, body],
        )?;

        // Insert entities
        tx.execute(
            "INSERT OR REPLACE INTO entities (rel_path, stem, domain, title, type, summary, status, repo, host, network, tags, updated, updated_by, metadata_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                rel, stem, domain, title, note_type, summary, status, repo, host, network, tags_str,
                updated, updated_by, metadata_json
            ],
        )?;

        // Frontmatter-declared relations keep the YAML key verbatim as the type,
        // so declared dependencies are visible to blast/map/contract.
        if let Some(Value::Object(declared)) = fm.get("relations") {
            for (relation_type, targets) in declared {
                match targets {
                    Value::Array(items) => {
                        for item in items {
                            let target = extract_target_stem(&scalar_text(item));
                            if !target.is_empty() {
                                tx.execute(
                                    "INSERT INTO relations (source_rel, target_stem, relation_type) VALUES (?1, ?2, ?3)",
                                    params![rel, target, relation_type],
                                )?;
                            }
                        }
                    }
                    other => {
                        let target = extract_target_stem(&scalar_text(other));
                        if !target.is_empty() {
                            tx.execute(
                                "INSERT INTO relations (source_rel, target_stem, relation_type) VALUES (?1, ?2, ?3)",
                                params![rel, target, relation_type],
                            )?;
                        }
                    }
                }
            }
        }

        // Body wikilinks resolve through note stems; escaped links (`\[[x]]`) are
        // documentation, not references.
        for clean_stem in parse_wikilinks(&body) {
            tx.execute(
                "INSERT INTO relations (source_rel, target_stem, relation_type) VALUES (?1, ?2, 'references')",
                params![rel, clean_stem],
            )?;
        }
        // Invariants: declared in frontmatter, or bulleted under an `Invariants`
        // heading.
        if let Some(Value::Array(rules)) = fm.get("invariants") {
            for rule in rules {
                let rule = scalar_text(rule);
                if !rule.is_empty() {
                    tx.execute(
                        "INSERT INTO invariants (source_rel, invariant_text) VALUES (?1, ?2)",
                        params![rel, rule],
                    )?;
                }
            }
        }
        let invariants_section = crate::storage::extract_section(&body, "Invariants")
            .or_else(|| crate::storage::extract_section(&body, "Non-Negotiable Invariants"));
        if let Some(section) = invariants_section {
            for line in section.lines() {
                if let Some(rule) = line.trim().strip_prefix("- ") {
                    let rule = rule.trim();
                    if !rule.is_empty() {
                        tx.execute(
                            "INSERT INTO invariants (source_rel, invariant_text) VALUES (?1, ?2)",
                            params![rel, rule],
                        )?;
                    }
                }
            }
        }

        // Extract bash:verify blocks
        for cap in verify_re.captures_iter(&body) {
            let cmd = cap[1].trim();
            if !cmd.is_empty() {
                tx.execute(
                    "INSERT INTO verifications (source_rel, command) VALUES (?1, ?2)",
                    params![rel, cmd],
                )?;
            }
        }

        // Services catalog table: host comes from this machine, not a literal.
        let default_host = crate::storage::machine_id();
        if rel.ends_with("Services-Catalog.md") {
            for line in body.lines() {
                let trimmed = line.trim();
                if !trimmed.starts_with('|')
                    || trimmed.contains(":---")
                    || trimmed.contains("Service / Stack")
                {
                    continue;
                }
                let raw_cols: Vec<&str> = trimmed.split('|').collect();
                if raw_cols.len() >= 6 {
                    let svc_name = raw_cols[1].replace(['*', '`'], "").trim().to_string();
                    let container = raw_cols[2].replace('`', "").trim().to_string();
                    let ports = raw_cols[3].trim().to_string();
                    let replicas = raw_cols[4].trim().to_string();
                    let role = raw_cols[5].trim().to_string();

                    if !svc_name.is_empty() {
                        tx.execute(
                            "INSERT INTO services (name, container_prefix, ports, host, network, replicas, role, rel_path) VALUES (?1, ?2, ?3, ?4, 'default', ?5, ?6, ?7)",
                            params![svc_name, container, ports, default_host, replicas, role, rel],
                        )?;
                    }
                }
            }
        }

        // Notes that declare `ports:` or are typed as a service are services too.
        if fm.get("ports").is_some() || note_type == "service" {
            let ports_val = match fm.get("ports") {
                Some(Value::Array(items)) => scalar_join(items),
                Some(Value::String(s)) => s.clone(),
                Some(other) => scalar_text(other),
                None => String::new(),
            };
            let container = fm
                .get("container")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_*", stem));
            let svc_host = host.clone().unwrap_or_else(|| default_host.clone());
            let svc_net = network.clone().unwrap_or_else(|| "default".to_string());

            tx.execute(
                "INSERT INTO services (name, container_prefix, ports, host, network, replicas, role, rel_path) VALUES (?1, ?2, ?3, ?4, ?5, '1', ?6, ?7)",
                params![stem, container, ports_val, svc_host, svc_net, summary, rel],
            )?;
        }

        // Explicit `services:` frontmatter block.
        if let Some(svcs) = fm.get("services").and_then(|v| v.as_array()) {
            for s in svcs {
                let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let container = s
                    .get("container")
                    .or_else(|| s.get("container_prefix"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let ports = s
                    .get("ports")
                    .and_then(|v| v.as_str())
                    .or_else(|| s.get("port").and_then(|v| v.as_str()))
                    .unwrap_or("");
                let s_host = s
                    .get("host")
                    .and_then(|v| v.as_str())
                    .unwrap_or(host.as_deref().unwrap_or(&default_host));
                let s_net = s
                    .get("network")
                    .and_then(|v| v.as_str())
                    .unwrap_or(network.as_deref().unwrap_or("default"));
                let replicas = s.get("replicas").and_then(|v| v.as_str()).unwrap_or("1");
                let role = s.get("role").and_then(|v| v.as_str()).unwrap_or("");

                if !name.is_empty() {
                    tx.execute(
                        "INSERT INTO services (name, container_prefix, ports, host, network, replicas, role, rel_path) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        params![name, container, ports, s_host, s_net, replicas, role, rel],
                    )?;
                }
            }
        }
    }

    tx.commit()?;

    let vectors = crate::vectors::sync_note_vectors(con, &vector_sources)?;

    let meta_tx = con.transaction()?;
    for file in &to_update {
        meta_tx.execute(
            "INSERT OR REPLACE INTO file_meta (rel_path, blake3_hash, size, indexed_at) VALUES (?1, ?2, ?3, ?4)",
            params![&file.rel_path, file.hash, file.size as i64, &now_iso],
        )?;
    }
    meta_tx.commit()?;

    Ok(SyncReport {
        added: added_count,
        updated: updated_count,
        deleted: deleted_count,
        unchanged: unchanged_count,
        total: total_count,
        parse_errors,
        vectors: Some(vectors),
        duration_ms: t0.elapsed().as_secs_f64() * 1000.0,
    })
}
