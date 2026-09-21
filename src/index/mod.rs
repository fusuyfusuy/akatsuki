//! Indexing engine: Parallel Blake3 Merkle scanner, SQLite WAL setup, and FTS5 synchronization.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use rayon::prelude::*;
use regex::Regex;
use rusqlite::{params, Connection};
use serde_json::Value;

use crate::storage::parse_frontmatter;

#[derive(Debug, Default, serde::Serialize)]
pub struct SyncReport {
    pub added: usize,
    pub updated: usize,
    pub deleted: usize,
    pub unchanged: usize,
    pub total: usize,
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

fn init_tables(con: &Connection) -> Result<()> {
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
            summary TEXT,
            status TEXT,
            repo TEXT,
            host TEXT,
            network TEXT,
            tags TEXT
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
            breadcrumb TEXT NOT NULL,
            preview TEXT NOT NULL,
            vector_blob BLOB NOT NULL,
            dim INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_entities_stem ON entities(stem);
        CREATE INDEX IF NOT EXISTS idx_entities_title ON entities(title);
        CREATE INDEX IF NOT EXISTS idx_services_name ON services(name);
        CREATE INDEX IF NOT EXISTS idx_services_rel ON services(rel_path);
        CREATE INDEX IF NOT EXISTS idx_relations_source ON relations(source_rel);
        CREATE INDEX IF NOT EXISTS idx_relations_target ON relations(target_stem);
        "#,
    )?;

    Ok(())
}

struct ScannedFile {
    rel_path: String,
    #[allow(dead_code)]
    abs_path: PathBuf,
    hash: String,
    size: usize,
    content: Option<String>,
}

pub fn sync_vault_index(vault: &Path, con: &mut Connection) -> Result<SyncReport> {
    let t0 = std::time::Instant::now();

    // 1. Gather all existing indexed hashes
    let mut indexed_hashes: HashMap<String, String> = HashMap::new();
    {
        let mut stmt = con.prepare("SELECT rel_path, blake3_hash FROM file_meta")?;
        let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
        for r in rows.flatten() {
            indexed_hashes.insert(r.0, r.1);
        }
    }

    // 2. Discover markdown files
    let ignored_dirs = [".git", ".akatsuki", ".venv", "node_modules", ".obsidian", "__pycache__", "_templates"];
    let walker = walkdir::WalkDir::new(vault)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !ignored_dirs.iter().any(|ig| *ig == name)
        });

    let paths: Vec<PathBuf> = walker
        .flatten()
        .filter(|e| e.file_type().is_file() && e.path().extension().map_or(false, |ext| ext == "md"))
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
                abs_path: abs.clone(),
                hash,
                size,
                content: Some(content),
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
    let total_count = unchanged_count + updated_count;

    if to_delete.is_empty() && to_update.is_empty() {
        return Ok(SyncReport {
            added: 0,
            updated: 0,
            deleted: 0,
            unchanged: unchanged_count,
            total: total_count,
            duration_ms: t0.elapsed().as_secs_f64() * 1000.0,
        });
    }

    // 5. Apply SQLite Transactions
    let tx = con.transaction()?;

    for del_rel in to_delete {
        tx.execute("DELETE FROM file_meta WHERE rel_path = ?1", params![del_rel])?;
        tx.execute("DELETE FROM notes_fts WHERE rel_path = ?1", params![del_rel])?;
        tx.execute("DELETE FROM entities WHERE rel_path = ?1", params![del_rel])?;
        tx.execute("DELETE FROM services WHERE rel_path = ?1", params![del_rel])?;
        tx.execute("DELETE FROM relations WHERE source_rel = ?1", params![del_rel])?;
        tx.execute("DELETE FROM invariants WHERE source_rel = ?1", params![del_rel])?;
        tx.execute("DELETE FROM verifications WHERE source_rel = ?1", params![del_rel])?;
        tx.execute("DELETE FROM note_vectors WHERE rel_path = ?1", params![del_rel])?;
    }

    let wikilink_re = Regex::new(r"\[\[([^\]\|]+)(?:\|[^\]]+)?\]\]")?;
    let verify_re = Regex::new(r"(?s)```bash:verify\s*\n(.*?)\n```")?;

    let now_iso = chrono::Utc::now().to_rfc3339();

    for file in to_update {
        let rel = &file.rel_path;
        let content = file.content.unwrap_or_default();
        let stem = Path::new(rel).file_stem().and_then(|s| s.to_str()).unwrap_or(rel);
        let domain = rel.split('/').next().unwrap_or("").to_string();

        let (fm, body) = parse_frontmatter(&content);

        let title = fm.get("title").and_then(|v| v.as_str()).unwrap_or(stem).to_string();
        let summary = fm.get("summary").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let status = fm.get("status").and_then(|v| v.as_str()).map(|s| s.to_string());
        let repo = fm.get("repo").and_then(|v| v.as_str()).map(|s| s.to_string());
        let host = fm.get("host").and_then(|v| v.as_str()).map(|s| s.to_string());
        let network = fm.get("network").and_then(|v| v.as_str()).map(|s| s.to_string());

        let tags_str = match fm.get("tags") {
            Some(Value::Array(arr)) => arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(" "),
            Some(Value::String(s)) => s.clone(),
            _ => String::new(),
        };

        // Clean prior records
        tx.execute("DELETE FROM notes_fts WHERE rel_path = ?1", params![rel])?;
        tx.execute("DELETE FROM entities WHERE rel_path = ?1", params![rel])?;
        tx.execute("DELETE FROM services WHERE rel_path = ?1", params![rel])?;
        tx.execute("DELETE FROM relations WHERE source_rel = ?1", params![rel])?;
        tx.execute("DELETE FROM invariants WHERE source_rel = ?1", params![rel])?;
        tx.execute("DELETE FROM verifications WHERE source_rel = ?1", params![rel])?;

        // Insert notes_fts
        tx.execute(
            "INSERT INTO notes_fts (rel_path, stem, domain, title, tags, summary, body) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![rel, stem, domain, title, tags_str, summary, body],
        )?;

        // Insert entities
        tx.execute(
            "INSERT OR REPLACE INTO entities (rel_path, stem, domain, title, summary, status, repo, host, network, tags) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![rel, stem, domain, title, summary, status, repo, host, network, tags_str],
        )?;

        // Extract wikilinks for relations
        for cap in wikilink_re.captures_iter(&body) {
            let target = cap[1].trim();
            let target_stem = target.split('#').next().unwrap_or(target).trim();
            let clean_stem = target_stem.strip_suffix(".md").unwrap_or(target_stem);
            if !clean_stem.is_empty() {
                tx.execute(
                    "INSERT INTO relations (source_rel, target_stem, relation_type) VALUES (?1, ?2, 'wikilink')",
                    params![rel, clean_stem],
                )?;
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

        // Parse services from Services-Catalog.md Markdown table
        if rel.ends_with("Services-Catalog.md") {
            for line in body.lines() {
                let trimmed = line.trim();
                if !trimmed.starts_with('|') || trimmed.contains(":---") || trimmed.contains("Service / Stack") {
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
                            "INSERT INTO services (name, container_prefix, ports, host, network, replicas, role, rel_path) VALUES (?1, ?2, ?3, 'TanriZarAtmaz', 'dokploy-network', ?4, ?5, ?6)",
                            params![svc_name, container, ports, replicas, role, rel],
                        )?;
                    }
                }
            }
        }

        // Parse services if present in frontmatter
        if let Some(svcs) = fm.get("services").and_then(|v| v.as_array()) {
            for s in svcs {
                let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let container = s.get("container").or_else(|| s.get("container_prefix")).and_then(|v| v.as_str()).unwrap_or("");
                let ports = s.get("ports").and_then(|v| v.as_str()).or_else(|| s.get("port").and_then(|v| v.as_str())).unwrap_or("");
                let s_host = s.get("host").and_then(|v| v.as_str()).unwrap_or(host.as_deref().unwrap_or(""));
                let s_net = s.get("network").and_then(|v| v.as_str()).unwrap_or(network.as_deref().unwrap_or(""));
                let replicas = s.get("replicas").and_then(|v| v.as_str()).unwrap_or("");
                let role = s.get("role").and_then(|v| v.as_str()).unwrap_or("");

                if !name.is_empty() {
                    tx.execute(
                        "INSERT INTO services (name, container_prefix, ports, host, network, replicas, role, rel_path) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        params![name, container, ports, s_host, s_net, replicas, role, rel],
                    )?;
                }
            }
        }

        // Update file_meta
        tx.execute(
            "INSERT OR REPLACE INTO file_meta (rel_path, blake3_hash, size, indexed_at) VALUES (?1, ?2, ?3, ?4)",
            params![rel, file.hash, file.size as i64, now_iso],
        )?;
    }

    tx.commit()?;

    Ok(SyncReport {
        added: updated_count,
        updated: updated_count,
        deleted: deleted_count,
        unchanged: unchanged_count,
        total: total_count,
        duration_ms: t0.elapsed().as_secs_f64() * 1000.0,
    })
}
