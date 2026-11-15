use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::limits::Limit;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
#[cfg(feature = "vectors")]
use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use crate::constants::{
    MAX_SEARCH_LIMIT, SQL_DEADLINE_SECS, SQL_MAX_COLUMNS, SQL_MAX_TOTAL_BYTES, SQL_MAX_VALUE_BYTES,
};
use crate::index::open_synced_db;
use crate::storage::{parse_frontmatter, read_note_utf8, resolve_note_file};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub rel_path: String,
    pub stem: String,
    pub domain: String,
    pub title: String,
    pub summary: String,
    pub score: f64,
    pub snippet: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub breadcrumb: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<GraphAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphAttachment {
    pub upstream: Vec<String>,
    pub downstream: Vec<String>,
    pub services: Vec<String>,
}

pub fn search_vault(
    vault: &Path,
    query: &str,
    domain: Option<&str>,
    limit: usize,
    with_graph: bool,
    mode: &str,
) -> Result<Vec<SearchHit>> {
    let clean_query = query.trim();
    if clean_query.is_empty() {
        return Ok(Vec::new());
    }

    let limit = limit.clamp(1, MAX_SEARCH_LIMIT);
    let con = open_synced_db(vault)?;

    let hits = match mode {
        "bm25" => run_bm25_search(&con, clean_query, domain, limit)?,
        // Explicit vector mode never silently becomes a keyword search: without
        // the feature or the weights it is an error the caller can act on.
        "vector" => crate::vectors::search_vectors(vault, clean_query, domain, limit)?,
        "hybrid" => {
            let bm25_limit = (limit * 2).max(20);
            let bm25_hits = run_bm25_search(&con, clean_query, domain, bm25_limit)?;

            #[cfg(feature = "vectors")]
            {
                if !crate::vectors::are_models_available() {
                    bm25_hits.into_iter().take(limit).collect()
                } else {
                    let vec_hits =
                        crate::vectors::search_vectors(vault, clean_query, domain, bm25_limit)
                            .unwrap_or_else(|e| {
                                eprintln!(
                                    "⚠ semantic ranking failed, using keyword ranking only: {e:#}"
                                );
                                Vec::new()
                            });
                    if vec_hits.is_empty() {
                        bm25_hits.into_iter().take(limit).collect()
                    } else {
                        fuse_rrf(bm25_hits, vec_hits, limit)
                    }
                }
            }
            #[cfg(not(feature = "vectors"))]
            {
                bm25_hits.into_iter().take(limit).collect()
            }
        }
        other => anyhow::bail!(
            "Invalid search mode '{}': expected 'hybrid', 'bm25', or 'vector'",
            other
        ),
    };

    if with_graph && !hits.is_empty() {
        return attach_graph_metadata(&con, hits);
    }

    Ok(hits)
}

/// Expands one query term the way the legacy engine did: prefix match plus a
/// suffix stem, so `caching` also finds `cache`.
fn expand_query_term(word: &str) -> Vec<String> {
    let word = word.to_lowercase();
    let mut variants = vec![word.clone()];

    if word.len() > 4 {
        variants.push(format!("{}*", word));
        for suffix in ["ing", "ed", "es", "s", "er", "able", "ive", "tion", "ment"] {
            if word.ends_with(suffix) && word.len() - suffix.len() >= 3 {
                let stem = &word[..word.len() - suffix.len()];
                variants.push(format!("{}*", stem));
                variants.push(stem.to_string());
                break;
            }
        }
    }

    let mut seen: Vec<String> = Vec::new();
    for variant in variants {
        if !seen.contains(&variant) {
            seen.push(variant);
        }
    }
    seen
}

fn build_fts_clause(query: &str) -> Vec<String> {
    static TOKEN_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static WORD_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let token_re =
        TOKEN_RE.get_or_init(|| Regex::new(r#""([^"]+)"|(\w+)"#).expect("valid token regex"));
    let word_re = WORD_RE.get_or_init(|| Regex::new(r"\w+").expect("valid word regex"));

    let mut terms: Vec<String> = Vec::new();
    for cap in token_re.captures_iter(query) {
        if let Some(phrase_match) = cap.get(1) {
            let phrase_words: Vec<&str> = word_re
                .find_iter(phrase_match.as_str())
                .map(|m| m.as_str())
                .collect();
            if !phrase_words.is_empty() {
                terms.push(format!("\"{}\"", phrase_words.join(" ")));
            }
        } else if let Some(word_match) = cap.get(2) {
            let variants = expand_query_term(word_match.as_str());
            if variants.len() == 1 {
                terms.push(variants[0].clone());
            } else {
                terms.push(format!("({})", variants.join(" OR ")));
            }
        }
    }

    if terms.is_empty() {
        return vec![];
    }

    let and_clause = terms.join(" AND ");
    if terms.len() > 1 {
        vec![and_clause, terms.join(" OR ")]
    } else {
        vec![and_clause]
    }
}

pub fn run_bm25_search(
    con: &Connection,
    query: &str,
    domain_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    let clauses = build_fts_clause(query);
    if clauses.is_empty() {
        return Ok(Vec::new());
    }

    let norm_domain = domain_filter
        .map(|d| d.trim().trim_end_matches('/'))
        .filter(|d| !d.is_empty());
    let domain_clause = if norm_domain.is_some() {
        "AND (domain = ?2 OR rel_path LIKE ?3 ESCAPE '\\')"
    } else {
        ""
    };

    let sql = format!(
        r#"
        SELECT rel_path, stem, domain, title, summary,
               bm25(notes_fts, 0, 0, 0, 10.0, 5.0, 5.0, 1.0) as score,
               snippet(notes_fts, 6, '**', '**', '...', 12) as snippet
        FROM notes_fts
        WHERE notes_fts MATCH ?1 {domain_clause}
        ORDER BY score
        LIMIT ?4
        "#
    );

    let mut stmt = con.prepare(&sql)?;

    let map_hit = |row: &rusqlite::Row| -> rusqlite::Result<SearchHit> {
        Ok(SearchHit {
            rel_path: row.get(0)?,
            stem: row.get(1)?,
            domain: row.get(2)?,
            title: row.get(3)?,
            summary: row.get(4)?,
            score: (row.get::<_, f64>(5)?.abs() * 1000.0).round() / 1000.0,
            snippet: row
                .get::<_, Option<String>>(6)?
                .unwrap_or_default()
                .trim()
                .to_string(),
            breadcrumb: None,
            graph: None,
        })
    };

    for q_clause in clauses {
        let results: Vec<SearchHit> = if let Some(dom) = norm_domain {
            let like_prefix = format!("{}/%", crate::graph::escape_like(dom));
            stmt.query_map(params![q_clause, dom, like_prefix, limit as i64], map_hit)?
                .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            stmt.query_map(params![q_clause, "", "", limit as i64], map_hit)?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };

        if !results.is_empty() {
            return Ok(results);
        }
    }

    Ok(Vec::new())
}

#[cfg(feature = "vectors")]
fn fuse_rrf(bm25_hits: Vec<SearchHit>, vec_hits: Vec<SearchHit>, limit: usize) -> Vec<SearchHit> {
    let k = 60.0;
    let mut rrf_scores: HashMap<String, f64> = HashMap::new();
    let mut hit_map: HashMap<String, SearchHit> = HashMap::new();

    for (rank, hit) in bm25_hits.into_iter().enumerate() {
        let score = 1.0 / (k + (rank as f64 + 1.0));
        *rrf_scores.entry(hit.rel_path.clone()).or_insert(0.0) += score;
        hit_map.insert(hit.rel_path.clone(), hit);
    }

    for (rank, hit) in vec_hits.into_iter().enumerate() {
        let score = 1.0 / (k + (rank as f64 + 1.0));
        *rrf_scores.entry(hit.rel_path.clone()).or_insert(0.0) += score;
        hit_map.entry(hit.rel_path.clone()).or_insert(hit);
    }

    let mut ranked: Vec<(String, f64)> = rrf_scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    ranked
        .into_iter()
        .take(limit)
        .filter_map(|(path, score)| {
            let mut hit = hit_map.remove(&path)?;
            hit.score = (score * 10000.0).round() / 10000.0;
            Some(hit)
        })
        .collect()
}

fn attach_graph_metadata(con: &Connection, mut hits: Vec<SearchHit>) -> Result<Vec<SearchHit>> {
    let mut stmt_up = con.prepare(
        "SELECT source_rel, relation_type FROM relations WHERE target_stem = ?1 LIMIT 5",
    )?;
    let mut stmt_down = con.prepare(
        "SELECT target_stem, relation_type FROM relations WHERE source_rel = ?1 LIMIT 5",
    )?;
    let mut stmt_svc = con.prepare("SELECT name, ports FROM services WHERE name = ?1 OR container_prefix = ?1 OR rel_path = ?2 LIMIT 3")?;

    for hit in &mut hits {
        let up_rows = stmt_up
            .query_map(params![hit.stem], |r| {
                Ok(format!(
                    "{} ({})",
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let down_rows = stmt_down
            .query_map(params![hit.rel_path], |r| {
                Ok(format!(
                    "{} ({})",
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let svc_rows = stmt_svc
            .query_map(params![hit.stem, hit.rel_path], |r| {
                let name = r.get::<_, String>(0)?;
                let ports = r.get::<_, Option<String>>(1)?.unwrap_or_default();
                if ports.is_empty() {
                    Ok(name)
                } else {
                    Ok(format!("{}:{}", name, ports))
                }
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        hit.graph = Some(GraphAttachment {
            upstream: up_rows,
            downstream: down_rows,
            services: svc_rows,
        });
    }

    Ok(hits)
}

pub fn format_hits_compact(hits: &[SearchHit]) -> String {
    if hits.is_empty() {
        return "No notes found matching query.".to_string();
    }

    let mut out = Vec::new();
    out.push(format!("Found {} matching note(s):", hits.len()));

    for h in hits {
        let snip = if h.snippet.is_empty() {
            h.summary.clone()
        } else {
            h.snippet.replace('\n', " ")
        };
        let snip_clean = ellipsize(&snip, 160);

        out.push(format!(
            "\n- **{}** (`{}`) [Score: {}]: {}\n  Snippet: {}",
            h.title, h.rel_path, h.score, h.summary, snip_clean
        ));

        if let Some(ref g) = h.graph {
            let mut parts = Vec::new();
            if !g.upstream.is_empty() {
                parts.push(format!("Up: {}", g.upstream.join(", ")));
            }
            if !g.downstream.is_empty() {
                parts.push(format!("Down: {}", g.downstream.join(", ")));
            }
            if !g.services.is_empty() {
                parts.push(format!("Svc: {}", g.services.join(", ")));
            }
            if !parts.is_empty() {
                out.push(format!("  Graph: {}", parts.join(" | ")));
            }
        }
    }

    out.join("\n")
}

/// Truncates on a character boundary so multi-byte text can never panic a slice.
fn ellipsize(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push_str("...");
    out
}

fn strip_leading_sql_comments(mut s: &str) -> &str {
    loop {
        s = s.trim_start();
        if s.starts_with("--") {
            if let Some(pos) = s.find('\n') {
                s = &s[pos + 1..];
            } else {
                return "";
            }
        } else if s.starts_with("/*") {
            if let Some(pos) = s.find("*/") {
                s = &s[pos + 2..];
            } else {
                return "";
            }
        } else {
            break;
        }
    }
    s
}

pub fn execute_sql_query(vault: &Path, sql: &str) -> Result<Vec<serde_json::Value>> {
    let stripped = strip_leading_sql_comments(sql).trim();
    let keyword = stripped
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    if !matches!(keyword.as_str(), "SELECT" | "WITH" | "EXPLAIN") {
        anyhow::bail!("Security violation: only SELECT, WITH and EXPLAIN queries are permitted.");
    }

    if has_multiple_sql_statements(stripped) {
        anyhow::bail!("Security violation: multiple statements are not permitted.");
    }

    // Reconcile first, then execute on a dedicated read-only + query_only connection.
    let _synced = open_synced_db(vault)?;
    let db_path = vault.join(".akatsuki/cache.db");
    let con = Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )?;
    con.busy_timeout(Duration::from_secs(5))?;
    con.pragma_update(None, "query_only", "ON")?;
    con.set_limit(Limit::SQLITE_LIMIT_LENGTH, SQL_MAX_VALUE_BYTES);
    con.set_limit(Limit::SQLITE_LIMIT_COLUMN, SQL_MAX_COLUMNS);

    let mut stmt = con.prepare(stripped)?;
    if !stmt.readonly() {
        anyhow::bail!("Security violation: query is not read-only.");
    }
    let col_names: Vec<String> = stmt
        .column_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    if col_names.len() > SQL_MAX_COLUMNS as usize {
        anyhow::bail!(
            "Query returns {} columns (maximum permitted is {})",
            col_names.len(),
            SQL_MAX_COLUMNS
        );
    }

    // Interrupt the query from a timer thread; dropping `done` cancels the timer.
    let interrupt = con.get_interrupt_handle();
    let (done, cancelled) = mpsc::channel::<()>();
    let timer = std::thread::spawn(move || {
        let timeout = Duration::from_secs(SQL_DEADLINE_SECS);
        if cancelled.recv_timeout(timeout) == Err(mpsc::RecvTimeoutError::Timeout) {
            interrupt.interrupt();
        }
    });
    let outcome = collect_rows(&mut stmt, &col_names);
    drop(done);
    let _ = timer.join();
    outcome.with_context(|| format!("query failed or was interrupted (limit {SQL_DEADLINE_SECS}s)"))
}

/// Comment- and quote-aware scanner returning `true` when `sql` contains more
/// than one non-empty SQL statement (`;` followed by non-comment tokens).
fn has_multiple_sql_statements(sql: &str) -> bool {
    let bytes = sql.as_bytes();
    let mut i = 0usize;
    let mut saw_semicolon = false;

    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b == b'-' && bytes.get(i + 1) == Some(&b'-') {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        if saw_semicolon {
            return true;
        }
        match b {
            b'\'' | b'"' | b'`' => {
                let quote = b;
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == quote {
                        if bytes.get(i + 1) == Some(&quote) {
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            b'[' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b']' {
                    i += 1;
                }
                i = (i + 1).min(bytes.len());
            }
            b';' => {
                saw_semicolon = true;
                i += 1;
            }
            _ => i += 1,
        }
    }
    false
}

fn collect_rows(
    stmt: &mut rusqlite::Statement<'_>,
    col_names: &[String],
) -> Result<Vec<serde_json::Value>> {
    const MAX_SQL_ROWS: usize = 500;
    let mut rows = stmt.query([])?;
    let mut results = Vec::new();
    let mut total_bytes = 0usize;

    while let Some(row) = rows.next()? {
        if results.len() >= MAX_SQL_ROWS {
            break;
        }
        let mut obj = serde_json::Map::new();
        for (i, name) in col_names.iter().enumerate() {
            let val_ref = row.get_ref(i)?;
            let (json_val, cell_bytes) = match val_ref {
                rusqlite::types::ValueRef::Null => (serde_json::Value::Null, 4),
                rusqlite::types::ValueRef::Integer(v) => (serde_json::Value::from(v), 8),
                rusqlite::types::ValueRef::Real(v) => (serde_json::Value::from(v), 8),
                rusqlite::types::ValueRef::Text(v) => {
                    let s = String::from_utf8_lossy(v).into_owned();
                    let len = s.len();
                    (serde_json::Value::String(s), len)
                }
                rusqlite::types::ValueRef::Blob(b) => {
                    let s = format!("<blob len={}>", b.len());
                    let len = s.len();
                    (serde_json::Value::String(s), len)
                }
            };
            total_bytes = total_bytes.saturating_add(name.len() + cell_bytes + 4);
            if total_bytes > SQL_MAX_TOTAL_BYTES {
                anyhow::bail!(
                    "Query result exceeds {} byte safety ceiling; narrow SELECT columns or add LIMIT",
                    SQL_MAX_TOTAL_BYTES
                );
            }
            obj.insert(name.clone(), json_val);
        }
        results.push(serde_json::Value::Object(obj));
    }

    Ok(results)
}

fn traverse_value_keypath(cur: &serde_json::Value, p: &str) -> Option<serde_json::Value> {
    match cur {
        serde_json::Value::Object(map) => map.get(p).cloned(),
        serde_json::Value::Array(arr) => {
            if let Ok(idx) = p.parse::<usize>() {
                arr.get(idx).cloned()
            } else {
                None
            }
        }
        _ => None,
    }
}

fn walk_value_tail(
    mut cur: serde_json::Value,
    tail: &[&str],
    context_label: &str,
) -> Result<serde_json::Value> {
    for p in tail {
        cur = traverse_value_keypath(&cur, p)
            .ok_or_else(|| anyhow::anyhow!("Property '{}' not found in {}", p, context_label))?;
    }
    Ok(cur)
}

fn split_keypath_segments(raw: &str) -> Vec<&str> {
    raw.split('.')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect()
}

pub fn get_keypath(vault: &Path, keypath: &str) -> Result<serde_json::Value> {
    let clean_key = keypath.trim();
    if clean_key.is_empty() {
        anyhow::bail!("Error: Empty keypath.");
    }

    let con = open_synced_db(vault)?;

    // 1. Longest-stem-first matching for "services."
    if let Some(rest) = clean_key.strip_prefix("services.") {
        let parts = split_keypath_segments(rest);
        if !parts.is_empty() {
            let mut stmt = con.prepare(
                "SELECT name, container_prefix, ports, host, network, replicas, role, rel_path \
                 FROM services WHERE name = ?1 OR container_prefix = ?1 LIMIT 1",
            )?;
            for i in (1..=parts.len()).rev() {
                let candidate_svc = parts[..i].join(".");
                let mut rows = stmt.query(params![candidate_svc])?;
                if let Some(row) = rows.next()? {
                    let obj = serde_json::json!({
                        "name": row.get::<_, String>(0)?,
                        "container_prefix": row.get::<_, String>(1)?,
                        "ports": row.get::<_, String>(2)?,
                        "host": row.get::<_, String>(3)?,
                        "network": row.get::<_, String>(4)?,
                        "replicas": row.get::<_, String>(5)?,
                        "role": row.get::<_, String>(6)?,
                        "rel_path": row.get::<_, String>(7)?,
                    });
                    return walk_value_tail(
                        obj,
                        &parts[i..],
                        &format!("service '{}'", candidate_svc),
                    );
                }
            }
        }
    }

    // 2. Longest-stem-first matching for "entities."
    if let Some(rest) = clean_key.strip_prefix("entities.") {
        let parts = split_keypath_segments(rest);
        if !parts.is_empty() {
            let mut stmt = con.prepare(
                "SELECT metadata_json FROM entities WHERE stem = ?1 OR rel_path = ?1 LIMIT 1",
            )?;
            for i in (1..=parts.len()).rev() {
                let candidate_stem = parts[..i].join(".");
                let mut rows = stmt.query(params![candidate_stem])?;
                if let Some(row) = rows.next()? {
                    let raw: String = row.get(0)?;
                    let cur: serde_json::Value = serde_json::from_str(&raw).with_context(|| {
                        format!(
                            "Entity '{}' carries unreadable metadata_json",
                            candidate_stem
                        )
                    })?;
                    return walk_value_tail(
                        cur,
                        &parts[i..],
                        &format!("entity '{}'", candidate_stem),
                    );
                }
            }
        }
    }

    // 3. Direct frontmatter lookup on note file (with longest-stem matching for dotted files)
    let parts = split_keypath_segments(clean_key);
    for i in (1..=parts.len()).rev() {
        let candidate_stem = parts[..i].join(".");
        if let Some(note_path) = resolve_note_file(vault, &candidate_stem) {
            let content = read_note_utf8(&note_path)?;
            let (fm, _) = parse_frontmatter(&content).with_context(|| {
                format!("Note '{}' has invalid frontmatter", note_path.display())
            })?;
            return walk_value_tail(
                fm,
                &parts[i..],
                &format!("frontmatter of '{}'", note_path.display()),
            );
        }
    }

    anyhow::bail!("Could not resolve keypath '{}'", keypath);
}

pub fn list_services(vault: &Path) -> Result<Vec<serde_json::Value>> {
    let con = open_synced_db(vault)?;
    let mut stmt = con.prepare(
        "SELECT name, container_prefix, ports, replicas, role, host, network FROM services",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(serde_json::json!({
                "name": r.get::<_, String>(0)?,
                "container_prefix": r.get::<_, String>(1)?,
                "ports": r.get::<_, String>(2)?,
                "replicas": r.get::<_, String>(3)?,
                "role": r.get::<_, String>(4)?,
                "host": r.get::<_, String>(5)?,
                "network": r.get::<_, String>(6)?,
            }))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn list_projects(vault: &Path) -> Result<Vec<serde_json::Value>> {
    let con = open_synced_db(vault)?;
    let mut stmt = con.prepare(
        "SELECT stem, title, status, repo, host, network, summary FROM entities \
         WHERE domain = '20-Projects' AND COALESCE(type, '') != 'moc' AND stem != 'Projects-MOC' AND stem NOT LIKE '%-MOC'",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(serde_json::json!({
                "stem": r.get::<_, String>(0)?,
                "title": r.get::<_, String>(1)?,
                "status": r.get::<_, Option<String>>(2)?,
                "repo": r.get::<_, Option<String>>(3)?,
                "host": r.get::<_, Option<String>>(4)?,
                "network": r.get::<_, Option<String>>(5)?,
                "summary": r.get::<_, Option<String>>(6)?,
            }))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn list_notes(vault: &Path, domain: Option<&str>) -> Result<Vec<serde_json::Value>> {
    let con = open_synced_db(vault)?;
    let base = "SELECT rel_path, stem, title, type, summary, status, metadata_json FROM entities";
    let sql = if domain.is_some() {
        format!(
            "{} WHERE domain = ?1 OR rel_path LIKE ?2 ESCAPE '\\' ORDER BY rel_path ASC",
            base
        )
    } else {
        format!("{} ORDER BY rel_path ASC", base)
    };

    let mut stmt = con.prepare(&sql)?;
    let map_row = |r: &rusqlite::Row| -> rusqlite::Result<serde_json::Value> {
        let metadata: Option<String> = r.get(6)?;
        let meta: serde_json::Value = metadata
            .as_deref()
            .and_then(|m| serde_json::from_str(m).ok())
            .unwrap_or_else(|| serde_json::json!({}));

        Ok(serde_json::json!({
            "rel_path": r.get::<_, String>(0)?,
            "stem": r.get::<_, String>(1)?,
            "title": r.get::<_, String>(2)?,
            "type": r.get::<_, String>(3)?,
            "summary": r.get::<_, Option<String>>(4)?.unwrap_or_default(),
            "status": r.get::<_, Option<String>>(5)?.unwrap_or_default(),
            "tags": meta.get("tags").cloned().unwrap_or_else(|| serde_json::json!([])),
        }))
    };

    let rows: Vec<serde_json::Value> = if let Some(dom) = domain {
        let clean = dom.trim().trim_end_matches('/');
        let escaped = crate::graph::escape_like(clean);
        stmt.query_map(rusqlite::params![clean, format!("{}/%", escaped)], map_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        stmt.query_map([], map_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };

    Ok(rows)
}
