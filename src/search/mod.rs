#[cfg(feature = "vectors")]
use std::collections::HashMap;
use std::path::Path;
use anyhow::Result;
use regex::Regex;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::index::{open_cache_db, sync_vault_index};

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
