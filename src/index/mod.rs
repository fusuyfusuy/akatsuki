//! Indexing engine: Parallel Blake3 Merkle scanner, SQLite WAL setup, and FTS5 synchronization.

use anyhow::{Context, Result};
use rayon::prelude::*;
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::storage::{body_after_frontmatter, parse_frontmatter, strip_fenced_code};

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
/// Escaped wikilinks (`\[[x]]`) and wikilinks inside code fences are ignored as documentation.
pub fn parse_wikilinks(body: &str) -> Vec<String> {
    use std::sync::OnceLock;
    static INLINE_RE: OnceLock<Regex> = OnceLock::new();
    static RE: OnceLock<Regex> = OnceLock::new();

    let inline_re =
        INLINE_RE.get_or_init(|| Regex::new(r"`+[^`\n]+`+").expect("valid inline regex"));
    let wikilink_re = RE.get_or_init(|| {
        Regex::new(r"\[\[([^\]\|]+)(?:\|[^\]]+)?\]\]").expect("valid wikilink regex")
    });

    let without_fences = strip_fenced_code(body);
    let clean = inline_re.replace_all(&without_fences, "");

    let mut targets = Vec::new();
    for caps in wikilink_re.captures_iter(&clean) {
        let whole = match caps.get(0) {
            Some(w) => w,
            None => continue,
        };
        let backslashes = clean[..whole.start()]
            .bytes()
            .rev()
            .take_while(|b| *b == b'\\')
            .count();
        if backslashes % 2 == 1 {
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
    if !vault.is_dir() {
        anyhow::bail!("Vault directory not found: {}", vault.display());
    }
    let ak_dir = vault.join(".akatsuki");
    fs::create_dir_all(&ak_dir)?;
    let db_path = ak_dir.join("cache.db");

    let con = Connection::open(&db_path)
        .with_context(|| format!("Failed to open database at {}", db_path.display()))?;

    // Performance pragmas & concurrency lock timeout
    con.busy_timeout(std::time::Duration::from_secs(5))?;
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
const SCHEMA_VERSION: &str = "0.2.3";

fn stored_schema_version(con: &Connection) -> Result<Option<String>> {
    Ok(con
        .query_row(
            "SELECT val FROM schema_meta WHERE key = 'version'",
            [],
            |r| r.get(0),
        )
        .optional()?)
}

/// Brings the projection schema to `SCHEMA_VERSION`. The cache is a pure
/// projection of the markdown vault, so a version bump rebuilds it wholesale
/// instead of migrating columns in place; the DROP + version stamp + CREATE run
/// in one IMMEDIATE transaction so a concurrent process never observes (or is
/// left with) a half-built schema. A warm, current cache takes no write lock.
fn init_tables(con: &Connection) -> Result<()> {
    con.execute_batch("CREATE TABLE IF NOT EXISTS schema_meta (key TEXT PRIMARY KEY, val TEXT);")?;

    if stored_schema_version(con)?.as_deref() == Some(SCHEMA_VERSION) {
        return create_tables(con);
    }

    con.execute_batch("BEGIN IMMEDIATE")?;
    let rebuilt = rebuild_schema(con);
    match rebuilt {
        Ok(()) => con.execute_batch("COMMIT")?,
        Err(e) => {
            let _ = con.execute_batch("ROLLBACK");
            return Err(e);
        }
    }
    Ok(())
}

/// Runs inside the IMMEDIATE transaction opened by `init_tables`; re-checks the
/// version because another process may have rebuilt while this one waited.
fn rebuild_schema(con: &Connection) -> Result<()> {
    if stored_schema_version(con)?.as_deref() != Some(SCHEMA_VERSION) {
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
    create_tables(con)
}

fn create_tables(con: &Connection) -> Result<()> {
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

#[derive(Clone)]
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

/// Inserts one relation edge unless this note already declared the same
/// `(target, type)` pair; empty targets are ignored.
fn insert_relation(
    tx: &Connection,
    seen: &mut HashSet<(String, String)>,
    source_rel: &str,
    target: &str,
    relation_type: &str,
) -> Result<()> {
    if target.is_empty() || !seen.insert((target.to_string(), relation_type.to_string())) {
        return Ok(());
    }
    tx.execute(
        "INSERT INTO relations (source_rel, target_stem, relation_type) VALUES (?1, ?2, ?3)",
        params![source_rel, target, relation_type],
    )?;
    Ok(())
}

fn load_indexed_hashes(con: &Connection) -> Result<HashMap<String, String>> {
    let mut stmt = con.prepare("SELECT rel_path, blake3_hash FROM file_meta")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn indexed_vector_paths(con: &Connection) -> Result<HashSet<String>> {
    let mut stmt = con.prepare("SELECT DISTINCT rel_path FROM note_vectors")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn delete_note_projections(tx: &Connection, rel: &str, clear_fts: bool) -> Result<()> {
    if clear_fts {
        tx.execute("DELETE FROM notes_fts WHERE rel_path = ?1", params![rel])?;
    }
    tx.execute("DELETE FROM note_vectors WHERE rel_path = ?1", params![rel])?;
    tx.execute("DELETE FROM entities WHERE rel_path = ?1", params![rel])?;
    tx.execute("DELETE FROM services WHERE rel_path = ?1", params![rel])?;
    tx.execute("DELETE FROM relations WHERE source_rel = ?1", params![rel])?;
    tx.execute("DELETE FROM invariants WHERE source_rel = ?1", params![rel])?;
    tx.execute(
        "DELETE FROM verifications WHERE source_rel = ?1",
        params![rel],
    )?;
    Ok(())
}

fn sync_vault_index_with(vault: &Path, con: &mut Connection, apply: bool) -> Result<SyncReport> {
    let t0 = std::time::Instant::now();

    // 1. Gather all existing indexed hashes
    let indexed_hashes = load_indexed_hashes(con)?;

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
        // The vault root itself may be named like an ignored directory.
        let name = e.file_name().to_string_lossy();
        e.depth() == 0 || !ignored_dirs.iter().any(|ig| *ig == name)
    });

    let mut paths: Vec<PathBuf> = Vec::new();
    // Unreadable directories and symlinked notes must never read as "deleted":
    // their indexed rows are retained until the filesystem answers authoritatively.
    let mut protected: Vec<String> = Vec::new();
    let mut walk_errors: Vec<String> = Vec::new();
    for entry in walker {
        match entry {
            Ok(e) if e.file_type().is_file() && e.path().extension().is_some_and(|x| x == "md") => {
                paths.push(e.into_path());
            }
            Ok(e) if e.path_is_symlink() && e.path().extension().is_some_and(|x| x == "md") => {
                if let Ok(rel) = e.path().strip_prefix(vault) {
                    protected.push(rel.to_string_lossy().to_string());
                }
            }
            Ok(_) => {}
            Err(err) => {
                if let Some(rel) = err.path().and_then(|p| p.strip_prefix(vault).ok()) {
                    let rel = rel.to_string_lossy().to_string();
                    walk_errors.push(format!("'{}': unreadable: {}", rel, err));
                    protected.push(rel);
                }
            }
        }
    }
    let is_protected = |rel: &str| {
        protected.iter().any(|p| {
            rel == p
                || rel
                    .strip_prefix(p.as_str())
                    .is_some_and(|r| r.starts_with('/'))
        })
    };

    // 3. Parallel Blake3 Merkle Scan via Rayon
    enum ScanOutcome {
        Ok(ScannedFile),
        Err(String, String),
    }

    let scan_results: Vec<ScanOutcome> = paths
        .par_iter()
        .filter_map(|abs| {
            let rel = abs.strip_prefix(vault).ok()?.to_string_lossy().to_string();
            match fs::read_to_string(abs) {
                Ok(content) => {
                    let hash = blake3::hash(content.as_bytes()).to_hex().to_string();
                    let size = content.len();
                    Some(ScanOutcome::Ok(ScannedFile {
                        rel_path: rel,
                        hash,
                        size,
                        content,
                    }))
                }
                Err(e) => Some(ScanOutcome::Err(rel, e.to_string())),
            }
        })
        .collect();

    let mut current_map: HashMap<String, ScannedFile> = HashMap::new();
    let mut read_errors: HashMap<String, String> = HashMap::new();

    for res in scan_results {
        match res {
            ScanOutcome::Ok(f) => {
                current_map.insert(f.rel_path.clone(), f);
            }
            ScanOutcome::Err(rel, err) => {
                read_errors.insert(rel, err);
            }
        }
    }

    let mut parse_errors: Vec<String> = Vec::new();
    for (rel, err) in &read_errors {
        parse_errors.push(format!("'{}': read error: {}", rel, err));
    }
    parse_errors.extend(walk_errors);
    parse_errors.sort();

    // 4. Calculate Diffs
    let mut to_delete = Vec::new();
    for rel in indexed_hashes.keys() {
        if !current_map.contains_key(rel) && !read_errors.contains_key(rel) && !is_protected(rel) {
            to_delete.push(rel.clone());
        }
    }

    let mut to_update = Vec::new();
    let mut unchanged_count = 0;

    for (rel, file) in &current_map {
        match indexed_hashes.get(rel) {
            Some(existing_hash) if existing_hash == &file.hash => {
                unchanged_count += 1;
                // Already-indexed notes that fail to parse stay reported on every sync.
                if let Err(e) = parse_frontmatter(&file.content) {
                    parse_errors.push(format!("'{}': {}", rel, e));
                }
            }
            _ => {
                to_update.push(file.clone());
            }
        }
    }

    for rel in read_errors.keys() {
        if indexed_hashes.contains_key(rel) {
            unchanged_count += 1;
        }
    }

    let deleted_count = to_delete.len();
    let updated_count = to_update.len();
    let added_count = to_update
        .iter()
        .filter(|f| !indexed_hashes.contains_key(&f.rel_path))
        .count();
    let total_count = unchanged_count + updated_count;

    if !apply {
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

    let mut missing_vectors = false;
    if crate::vectors::feature_enabled() {
        let with_vecs = indexed_vector_paths(con)?;
        if current_map.keys().any(|rel| !with_vecs.contains(rel)) {
            missing_vectors = true;
        }
    }

    if to_delete.is_empty() && to_update.is_empty() && !missing_vectors {
        return Ok(SyncReport {
            added: 0,
            updated: 0,
            deleted: 0,
            unchanged: unchanged_count,
            total: total_count,
            parse_errors,
            vectors: None,
            duration_ms: t0.elapsed().as_secs_f64() * 1000.0,
        });
    }

    // 5. Apply SQLite Transactions
    let tx = con.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

    // Another process may have synced while this one waited for the write lock.
    // Apply only rows whose stored hash is still the one this scan started from;
    // anything else was already advanced by that process and must not be rolled
    // back to the (now stale) bytes read before the lock.
    let current_hashes = load_indexed_hashes(&tx)?;
    to_delete.retain(|rel| current_hashes.get(rel) == indexed_hashes.get(rel));
    to_update.retain(|f| current_hashes.get(&f.rel_path) == indexed_hashes.get(&f.rel_path));

    for del_rel in to_delete {
        tx.execute(
            "DELETE FROM file_meta WHERE rel_path = ?1",
            params![del_rel],
        )?;
        delete_note_projections(&tx, &del_rel, true)?;
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
                (
                    serde_json::json!({}),
                    body_after_frontmatter(content)
                        .trim_start_matches('\n')
                        .to_string(),
                )
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

        // Clean prior records. `rel_path` is UNINDEXED in the FTS table, so each
        // DELETE scans it: skip it for notes that were never indexed. Old vectors
        // describe old text, so drop them even when embedding is skipped on this run.
        delete_note_projections(&tx, rel, indexed_hashes.contains_key(rel))?;

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

        let mut seen_relations: HashSet<(String, String)> = HashSet::new();

        // Frontmatter-declared relations keep the YAML key verbatim as the type,
        // so declared dependencies are visible to blast/map/contract.
        if let Some(Value::Object(declared)) = fm.get("relations") {
            for (relation_type, targets) in declared {
                match targets {
                    Value::Array(items) => {
                        for item in items {
                            let target = extract_target_stem(&scalar_text(item));
                            insert_relation(&tx, &mut seen_relations, rel, &target, relation_type)?;
                        }
                    }
                    other => {
                        let target = extract_target_stem(&scalar_text(other));
                        insert_relation(&tx, &mut seen_relations, rel, &target, relation_type)?;
                    }
                }
            }
        }

        // Body wikilinks resolve through note stems; escaped links (`\[[x]]`) are
        // documentation, not references.
        for clean_stem in parse_wikilinks(&body) {
            insert_relation(&tx, &mut seen_relations, rel, &clean_stem, "references")?;
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
                let mut raw_cols: Vec<String> = Vec::new();
                let mut col = String::new();
                let mut chars = trimmed.chars().peekable();
                while let Some(c) = chars.next() {
                    if c == '\\' && chars.peek() == Some(&'|') {
                        col.push('|');
                        chars.next();
                    } else if c == '|' {
                        raw_cols.push(col.trim().to_string());
                        col.clear();
                    } else {
                        col.push(c);
                    }
                }
                raw_cols.push(col.trim().to_string());

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
                    .or_else(|| s.get("port"))
                    .map(|v| match v {
                        Value::Array(items) => scalar_join(items),
                        Value::String(s) => s.clone(),
                        other => scalar_text(other),
                    })
                    .unwrap_or_default();
                let s_host = s
                    .get("host")
                    .and_then(|v| v.as_str())
                    .unwrap_or(host.as_deref().unwrap_or(&default_host));
                let s_net = s
                    .get("network")
                    .and_then(|v| v.as_str())
                    .unwrap_or(network.as_deref().unwrap_or("default"));
                let replicas = s
                    .get("replicas")
                    .map(|v| match v {
                        Value::String(s) => s.clone(),
                        other => scalar_text(other),
                    })
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "1".to_string());
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

    for file in &to_update {
        tx.execute(
            "INSERT OR REPLACE INTO file_meta (rel_path, blake3_hash, size, indexed_at) VALUES (?1, ?2, ?3, ?4)",
            params![&file.rel_path, file.hash, file.size as i64, &now_iso],
        )?;
    }

    tx.commit()?;

    // Embeddings run in their own transaction after the lexical projection is
    // durable: an embedder failure must never roll back text rows or poison every
    // read command, and is reported (ADR-0002) instead of swallowed.
    if crate::vectors::feature_enabled() {
        let existing_vector_rels = indexed_vector_paths(con)?;
        for (rel, file) in &current_map {
            if !existing_vector_rels.contains(rel) && !vector_sources.iter().any(|(r, _)| r == rel)
            {
                vector_sources.push((rel.clone(), file.content.clone()));
            }
        }
    }
    vector_sources.sort_by(|a, b| a.0.cmp(&b.0));

    let vectors = crate::vectors::sync_note_vectors(con, &vector_sources).unwrap_or_else(|e| {
        format!("failed: {e:#} (text index is current; vectors will be retried)")
    });

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
