#[cfg(feature = "vectors")]
use std::collections::HashMap;
use std::path::Path;
use anyhow::Result;
use regex::Regex;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::index::{open_cache_db, sync_vault_index};
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

    let mut con = open_cache_db(vault)?;
    let _ = sync_vault_index(vault, &mut con)?;

    let hits = match mode {
        "bm25" => run_bm25_search(&con, clean_query, domain, limit)?,
        "vector" => {
            #[cfg(feature = "vectors")]
            {
                crate::vectors::search_vectors(vault, clean_query, domain, limit)?
            }
            #[cfg(not(feature = "vectors"))]
            {
                run_bm25_search(&con, clean_query, domain, limit)?
            }
        }
        _ /* hybrid */ => {
            let bm25_limit = (limit * 2).max(15);
            let bm25_hits = run_bm25_search(&con, clean_query, domain, bm25_limit)?;

            #[cfg(feature = "vectors")]
            {
                if let Ok(vec_hits) = crate::vectors::search_vectors(vault, clean_query, domain, bm25_limit) {
                    if !vec_hits.is_empty() {
                        fuse_rrf(bm25_hits, vec_hits, limit)
                    } else {
                        bm25_hits.into_iter().take(limit).collect()
                    }
                } else {
                    bm25_hits.into_iter().take(limit).collect()
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

    let and_clause = words
        .iter()
        .map(|w| format!("{}*", w))
        .collect::<Vec<_>>()
        .join(" AND ");

    let or_clause = words
        .iter()
        .map(|w| format!("{}*", w))
        .collect::<Vec<_>>()
        .join(" OR ");

    if words.len() > 1 {
        vec![and_clause, or_clause]
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

    let domain_clause = if domain_filter.is_some() {
        "AND domain = ?2"
    } else {
        ""
    };

    let sql = format!(
        r#"
        SELECT rel_path, stem, domain, title, summary,
               bm25(notes_fts, 0, 0, 0, 10.0, 5.0, 5.0, 1.0) as score,
               snippet(notes_fts, 6, '**', '**', '...', 10) as snippet
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
            score: row.get::<_, f64>(5)?.abs(),
            snippet: row.get::<_, Option<String>>(6)?.unwrap_or_default().trim().to_string(),
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
    let mut stmt_up = con.prepare("SELECT source_rel, relation_type FROM relations WHERE target_stem = ?1 LIMIT 5")?;
    let mut stmt_down = con.prepare("SELECT target_stem, relation_type FROM relations WHERE source_rel = ?1 LIMIT 5")?;
    let mut stmt_svc = con.prepare("SELECT name, ports FROM services WHERE name = ?1 OR container_prefix = ?1 OR rel_path = ?2 LIMIT 3")?;

    for hit in &mut hits {
        let up_rows = stmt_up
            .query_map(params![hit.stem], |r| Ok(format!("{} ({})", r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .filter_map(Result::ok)
            .collect();

        let down_rows = stmt_down
            .query_map(params![hit.rel_path], |r| Ok(format!("{} ({})", r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
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
        let snip_clean = if snip.len() > 160 {
            format!("{}...", &snip[..160])
        } else {
            snip
        };

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

pub fn execute_sql_query(vault: &Path, sql: &str) -> Result<Vec<serde_json::Value>> {
    let trimmed = sql.trim();
    if !trimmed.to_uppercase().starts_with("SELECT") {
        anyhow::bail!("Security violation: Only SELECT queries are permitted.");
    }

    let con = open_cache_db(vault)?;
    let mut stmt = con.prepare(trimmed)?;
    let col_names: Vec<String> = stmt.column_names().into_iter().map(|s| s.to_string()).collect();

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

pub fn get_keypath(vault: &Path, keypath: &str) -> Result<serde_json::Value> {
    let parts: Vec<&str> = keypath.split('.').map(|p| p.trim()).filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        anyhow::bail!("Error: Empty keypath.");
    }

    let category = parts[0];
    let con = open_cache_db(vault)?;

    if category == "services" && parts.len() >= 2 {
        let svc_name = parts[1];
        let pattern = format!("{}%", svc_name);
        let mut stmt = con.prepare("SELECT name, container_prefix, ports, host, network, replicas, role, rel_path FROM services WHERE name = ?1 OR name LIKE ?2 LIMIT 1")?;
        let mut rows = stmt.query(params![svc_name, pattern])?;
        if let Some(row) = rows.next()? {
            let mut obj = serde_json::Map::new();
            obj.insert("name".to_string(), serde_json::Value::String(row.get(0)?));
            obj.insert("container_prefix".to_string(), serde_json::Value::String(row.get(1)?));
            obj.insert("ports".to_string(), serde_json::Value::String(row.get(2)?));
            obj.insert("host".to_string(), serde_json::Value::String(row.get(3)?));
            obj.insert("network".to_string(), serde_json::Value::String(row.get(4)?));
            obj.insert("replicas".to_string(), serde_json::Value::String(row.get(5)?));
            obj.insert("role".to_string(), serde_json::Value::String(row.get(6)?));
            obj.insert("rel_path".to_string(), serde_json::Value::String(row.get(7)?));

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
        let mut stmt = con.prepare("SELECT rel_path, stem, domain, title, summary, status, repo, host, network, tags FROM entities WHERE stem = ?1 OR rel_path = ?1 LIMIT 1")?;
        let mut rows = stmt.query(params![ent_stem])?;
        if let Some(row) = rows.next()? {
            let mut obj = serde_json::Map::new();
            obj.insert("rel_path".to_string(), serde_json::Value::String(row.get(0)?));
            obj.insert("stem".to_string(), serde_json::Value::String(row.get(1)?));
            obj.insert("domain".to_string(), serde_json::Value::String(row.get(2)?));
            obj.insert("title".to_string(), serde_json::Value::String(row.get(3)?));
            obj.insert("summary".to_string(), row.get::<_, Option<String>>(4)?.map(serde_json::Value::String).unwrap_or(serde_json::Value::Null));
            obj.insert("status".to_string(), row.get::<_, Option<String>>(5)?.map(serde_json::Value::String).unwrap_or(serde_json::Value::Null));
            obj.insert("repo".to_string(), row.get::<_, Option<String>>(6)?.map(serde_json::Value::String).unwrap_or(serde_json::Value::Null));
            obj.insert("host".to_string(), row.get::<_, Option<String>>(7)?.map(serde_json::Value::String).unwrap_or(serde_json::Value::Null));
            obj.insert("network".to_string(), row.get::<_, Option<String>>(8)?.map(serde_json::Value::String).unwrap_or(serde_json::Value::Null));
            obj.insert("tags".to_string(), row.get::<_, Option<String>>(9)?.map(serde_json::Value::String).unwrap_or(serde_json::Value::Null));

            if parts.len() == 2 {
                return Ok(serde_json::Value::Object(obj));
            }

            let mut cur = serde_json::Value::Object(obj);
            for p in &parts[2..] {
                if let Some(next) = cur.get(*p) {
                    cur = next.clone();
                } else {
                    anyhow::bail!("Property '{}' not found in entity '{}'", p, ent_stem);
                }
            }
            return Ok(cur);
        } else {
            anyhow::bail!("Entity '{}' not found in vault index", ent_stem);
        }
    }

    // Frontmatter lookup on note file
    if let Some(note_path) = resolve_note_file(vault, category) {
        let content = std::fs::read_to_string(&note_path)?;
        let (fm, _) = parse_frontmatter(&content);
        if parts.len() == 1 {
            return Ok(fm);
        }
        let mut cur = fm;
        for p in &parts[1..] {
            if let Some(next) = cur.get(*p) {
                cur = next.clone();
            } else {
                anyhow::bail!("Key '{}' not found in frontmatter of '{}'", p, note_path.display());
            }
        }
        return Ok(cur);
    }

    anyhow::bail!("Could not resolve keypath '{}'", keypath);
}

pub fn list_notes(vault: &Path, domain: Option<&str>) -> Result<Vec<serde_json::Value>> {
    let con = open_cache_db(vault)?;
    let (sql, is_filtered) = if domain.is_some() {
        ("SELECT stem, rel_path, domain, title, summary, status FROM entities WHERE domain = ?1 ORDER BY stem", true)
    } else {
        ("SELECT stem, rel_path, domain, title, summary, status FROM entities ORDER BY domain, stem", false)
    };

    let mut stmt = con.prepare(sql)?;
    let map_row = |r: &rusqlite::Row| {
        Ok(serde_json::json!({
            "stem": r.get::<_, String>(0)?,
            "rel_path": r.get::<_, String>(1)?,
            "domain": r.get::<_, String>(2)?,
            "title": r.get::<_, String>(3)?,
            "summary": r.get::<_, Option<String>>(4)?,
            "status": r.get::<_, Option<String>>(5)?,
        }))
    };

    let rows: Vec<serde_json::Value> = if is_filtered {
        stmt.query_map(rusqlite::params![domain.unwrap()], map_row)?
            .filter_map(Result::ok)
            .collect()
    } else {
        stmt.query_map([], map_row)?
            .filter_map(Result::ok)
            .collect()
    };

    Ok(rows)
}


