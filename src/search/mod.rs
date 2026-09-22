use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
#[cfg(feature = "vectors")]
use std::collections::HashMap;
use std::path::Path;

use crate::index::open_synced_db;
use crate::storage::{parse_frontmatter, resolve_note_file};

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

    let con = open_synced_db(vault)?;

    let hits = match mode {
        "bm25" => run_bm25_search(&con, clean_query, domain, limit)?,
        // Explicit vector mode never silently becomes a keyword search: without
        // the feature or the weights it is an error the caller can act on.
        "vector" => crate::vectors::search_vectors(vault, clean_query, domain, limit)?,
        _ /* hybrid */ => {
            let bm25_limit = (limit * 2).max(20);
            let bm25_hits = run_bm25_search(&con, clean_query, domain, bm25_limit)?;

            #[cfg(feature = "vectors")]
            {
                // Semantic hits are a bonus, never a silent substitution: when they
                // are unavailable the caller is told through `vectors::hybrid_note`.
                let vec_hits = crate::vectors::search_vectors(vault, clean_query, domain, bm25_limit)
                    .unwrap_or_default();
                if vec_hits.is_empty() {
                    bm25_hits.into_iter().take(limit).collect()
                } else {
                    fuse_rrf(bm25_hits, vec_hits, limit)
                }
            }
            #[cfg(not(feature = "vectors"))]
            {
                bm25_hits.into_iter().take(limit).collect()
            }
        }
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
    let word_re = Regex::new(r"\w+").unwrap();
    let words: Vec<String> = word_re
        .find_iter(query)
        .map(|m| m.as_str().to_string())
        .collect();

    if words.is_empty() {
        return vec![];
    }

    if query.starts_with('"') && query.ends_with('"') && query.len() > 2 {
        let phrase = query.trim_matches('"').replace('"', "\"\"");
        return vec![format!("\"{}\"", phrase)];
    }

    let clause = |op: &str| {
        words
            .iter()
            .map(|w| {
                let variants = expand_query_term(w);
                if variants.len() == 1 {
                    variants[0].clone()
                } else {
                    format!("({})", variants.join(" OR "))
                }
            })
            .collect::<Vec<_>>()
            .join(&format!(" {} ", op))
    };

    if words.len() > 1 {
        vec![clause("AND"), clause("OR")]
    } else {
        vec![clause("AND")]
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

    let domain_clause = if domain_filter.is_some() {
        "AND domain = ?2"
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
        LIMIT ?3
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
        let results: Vec<SearchHit> = if let Some(dom) = domain_filter {
            stmt.query_map(params![q_clause, dom, limit as i64], map_hit)?
                .filter_map(Result::ok)
                .collect()
        } else {
            stmt.query_map(params![q_clause, "", limit as i64], map_hit)?
                .filter_map(Result::ok)
                .collect()
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
            .filter_map(Result::ok)
            .collect();

        let down_rows = stmt_down
            .query_map(params![hit.rel_path], |r| {
                Ok(format!(
                    "{} ({})",
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?
                ))
            })?
            .filter_map(Result::ok)
            .collect();

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
            .filter_map(Result::ok)
            .collect();

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
    let stripped = strip_leading_sql_comments(sql);
    let trimmed = stripped.trim().trim_end_matches(';').trim();
    let keyword = trimmed
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    if !matches!(keyword.as_str(), "SELECT" | "WITH" | "EXPLAIN") {
        anyhow::bail!("Security violation: only SELECT, WITH and EXPLAIN queries are permitted.");
    }

    let con = open_synced_db(vault)?;
    let mut stmt = con.prepare(trimmed)?;
    if !stmt.readonly() {
        anyhow::bail!("Security violation: query is not read-only.");
    }
    let col_names: Vec<String> = stmt
        .column_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect();

    let mut rows = stmt.query([])?;
    let mut results = Vec::new();

    while let Some(row) = rows.next()? {
        let mut obj = serde_json::Map::new();
        for (i, name) in col_names.iter().enumerate() {
            let val_ref = row.get_ref(i)?;
            let json_val = match val_ref {
                rusqlite::types::ValueRef::Null => serde_json::Value::Null,
                rusqlite::types::ValueRef::Integer(v) => serde_json::Value::from(v),
                rusqlite::types::ValueRef::Real(v) => serde_json::Value::from(v),
                rusqlite::types::ValueRef::Text(v) => {
                    let s = String::from_utf8_lossy(v);
                    if let Ok(nested) = serde_json::from_str::<serde_json::Value>(&s) {
                        nested
                    } else {
                        serde_json::Value::String(s.to_string())
                    }
                }
                rusqlite::types::ValueRef::Blob(b) => {
                    serde_json::Value::String(format!("<blob len={}>", b.len()))
                }
            };
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

pub fn get_keypath(vault: &Path, keypath: &str) -> Result<serde_json::Value> {
    let parts: Vec<&str> = keypath
        .split('.')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        anyhow::bail!("Error: Empty keypath.");
    }

    let category = parts[0];
    let con = open_synced_db(vault)?;

    if category == "services" && parts.len() >= 2 {
        let svc_name = parts[1];
        let pattern = format!("{}%", svc_name);
        let mut stmt = con.prepare("SELECT name, container_prefix, ports, host, network, replicas, role, rel_path FROM services WHERE name = ?1 OR name LIKE ?2 LIMIT 1")?;
        let mut rows = stmt.query(params![svc_name, pattern])?;
        if let Some(row) = rows.next()? {
            let mut obj = serde_json::Map::new();
            obj.insert("name".to_string(), serde_json::Value::String(row.get(0)?));
            obj.insert(
                "container_prefix".to_string(),
                serde_json::Value::String(row.get(1)?),
            );
            obj.insert("ports".to_string(), serde_json::Value::String(row.get(2)?));
            obj.insert("host".to_string(), serde_json::Value::String(row.get(3)?));
            obj.insert(
                "network".to_string(),
                serde_json::Value::String(row.get(4)?),
            );
            obj.insert(
                "replicas".to_string(),
                serde_json::Value::String(row.get(5)?),
            );
            obj.insert("role".to_string(), serde_json::Value::String(row.get(6)?));
            obj.insert(
                "rel_path".to_string(),
                serde_json::Value::String(row.get(7)?),
            );

            if parts.len() == 2 {
                return Ok(serde_json::Value::Object(obj));
            }
            let prop = parts[2];
            if let Some(val) = obj.get(prop) {
                return Ok(val.clone());
            } else {
                anyhow::bail!("Property '{}' not found in service '{}'", prop, svc_name);
            }
        } else {
            anyhow::bail!("Service '{}' not found in services catalog", svc_name);
        }
    }

    if category == "entities" && parts.len() >= 2 {
        let ent_stem = parts[1];
        let mut stmt = con.prepare(
            "SELECT metadata_json FROM entities WHERE stem = ?1 OR rel_path = ?1 LIMIT 1",
        )?;
        let mut rows = stmt.query(params![ent_stem])?;
        if let Some(row) = rows.next()? {
            let raw: String = row.get(0)?;
            let mut cur: serde_json::Value = serde_json::from_str(&raw).with_context(|| {
                format!("Entity '{}' carries unreadable metadata_json", ent_stem)
            })?;

            for p in &parts[2..] {
                if let Some(next) = traverse_value_keypath(&cur, p) {
                    cur = next;
                } else {
                    anyhow::bail!("Property '{}' not found in entity '{}'", p, ent_stem);
                }
            }
            return Ok(cur);
        }
        anyhow::bail!("Entity '{}' not found in vault index", ent_stem);
    }

    // Frontmatter lookup on note file
    if let Some(note_path) = resolve_note_file(vault, category) {
        let content = std::fs::read_to_string(&note_path)?;
        let (fm, _) = parse_frontmatter(&content)
            .with_context(|| format!("Note '{}' has invalid frontmatter", note_path.display()))?;
        if parts.len() == 1 {
            return Ok(fm);
        }
        let mut cur = fm;
        for p in &parts[1..] {
            if let Some(next) = traverse_value_keypath(&cur, p) {
                cur = next;
            } else {
                anyhow::bail!(
                    "Key '{}' not found in frontmatter of '{}'",
                    p,
                    note_path.display()
                );
            }
        }
        return Ok(cur);
    }

    anyhow::bail!("Could not resolve keypath '{}'", keypath);
}

pub fn list_notes(vault: &Path, domain: Option<&str>) -> Result<Vec<serde_json::Value>> {
    let con = open_synced_db(vault)?;
    let base = "SELECT rel_path, stem, title, type, summary, status, metadata_json FROM entities";
    let sql = if domain.is_some() {
        format!(
            "{} WHERE domain = ?1 OR rel_path LIKE ?2 ORDER BY rel_path ASC",
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
        stmt.query_map(rusqlite::params![clean, format!("{}/%", clean)], map_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        stmt.query_map([], map_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };

    Ok(rows)
}
