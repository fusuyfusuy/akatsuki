//! Knowledge graph, blast radius calculation, map traversal, and contract extraction.

use std::collections::{HashSet, VecDeque};
use std::path::Path;
use anyhow::{Context, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::index::{open_cache_db, sync_vault_index};
use crate::storage::{parse_frontmatter, resolve_note_file};

#[derive(Debug, Serialize, Deserialize)]
pub struct BlastRadius {
    pub target: String,
    pub upstream: Vec<RelationItem>,
    pub downstream: Vec<RelationItem>,
    pub boundary_sinks: Vec<ServiceItem>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RelationItem {
    pub source_or_target: String,
    pub relation_type: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServiceItem {
    pub name: String,
    pub container: String,
    pub ports: String,
    pub host: String,
    pub network: String,
    pub rel_path: String,
}

pub fn calculate_blast_radius(vault: &Path, target: &str) -> Result<BlastRadius> {
    let mut con = open_cache_db(vault)?;
    let _ = sync_vault_index(vault, &mut con)?;

    let clean_target = target.trim().trim_end_matches(".md");
    let stem = Path::new(clean_target).file_stem().and_then(|s| s.to_str()).unwrap_or(clean_target);

    // Upstream dependents
    let mut stmt_up = con.prepare("SELECT source_rel, relation_type FROM relations WHERE target_stem = ?1")?;
    let upstream: Vec<RelationItem> = stmt_up
        .query_map(params![stem], |r| {
            Ok(RelationItem {
                source_or_target: r.get(0)?,
                relation_type: r.get(1)?,
            })
        })?
        .filter_map(Result::ok)
        .collect();

    // Downstream dependencies
    let like_pattern = format!("%/{}%", stem);
    let exact_rel = format!("{}.md", stem);
    let mut stmt_down = con.prepare("SELECT target_stem, relation_type FROM relations WHERE source_rel = ?1 OR source_rel LIKE ?2")?;
    let downstream: Vec<RelationItem> = stmt_down
        .query_map(params![exact_rel, like_pattern], |r| {
            Ok(RelationItem {
                source_or_target: r.get(0)?,
                relation_type: r.get(1)?,
            })
        })?
        .filter_map(Result::ok)
        .collect();

    // Boundary sinks
    let mut stmt_svc = con.prepare("SELECT name, container_prefix, ports, host, network, rel_path FROM services WHERE name = ?1 OR container_prefix = ?1 OR rel_path LIKE ?2")?;
    let boundary_sinks: Vec<ServiceItem> = stmt_svc
        .query_map(params![stem, like_pattern], |r| {
            Ok(ServiceItem {
                name: r.get(0)?,
                container: r.get(1)?,
                ports: r.get(2)?,
                host: r.get(3)?,
                network: r.get(4)?,
                rel_path: r.get(5)?,
            })
        })?
        .filter_map(Result::ok)
        .collect();

    Ok(BlastRadius {
        target: stem.to_string(),
        upstream,
        downstream,
        boundary_sinks,
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NoteContract {
    pub stem: String,
    pub rel_path: String,
    pub title: String,
    pub summary: String,
    pub status: Option<String>,
    pub repo: Option<String>,
    pub host: Option<String>,
    pub network: Option<String>,
    pub ports: Vec<String>,
    pub dependencies: Vec<String>,
    pub dependents: Vec<String>,
    pub verifications: Vec<String>,
}

pub fn extract_contract(vault: &Path, note_query: &str) -> Result<NoteContract> {
    let note_path = resolve_note_file(vault, note_query)
        .with_context(|| format!("Note '{}' not found in vault", note_query))?;

    let content = std::fs::read_to_string(&note_path)?;
    let (fm, _) = parse_frontmatter(&content);

    let stem = note_path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
    let rel_path = note_path.strip_prefix(vault).unwrap_or(&note_path).to_string_lossy().to_string();

    let title = fm.get("title").and_then(|v| v.as_str()).unwrap_or(&stem).to_string();
    let summary = fm.get("summary").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let status = fm.get("status").and_then(|v| v.as_str()).map(|s| s.to_string());
    let repo = fm.get("repo").and_then(|v| v.as_str()).map(|s| s.to_string());
    let host = fm.get("host").and_then(|v| v.as_str()).map(|s| s.to_string());
    let network = fm.get("network").and_then(|v| v.as_str()).map(|s| s.to_string());

    let blast = calculate_blast_radius(vault, &stem)?;

    let mut ports = Vec::new();
    for svc in &blast.boundary_sinks {
        if !svc.ports.is_empty() {
            ports.push(format!("{}: {}", svc.name, svc.ports));
        }
    }

    let dependencies: Vec<String> = blast.downstream.into_iter().map(|d| d.source_or_target).collect();
    let dependents: Vec<String> = blast.upstream.into_iter().map(|u| u.source_or_target).collect();

    let con = open_cache_db(vault)?;
    let mut stmt = con.prepare("SELECT command FROM verifications WHERE source_rel = ?1")?;
    let verifications: Vec<String> = stmt
        .query_map(params![rel_path], |r| r.get(0))?
        .filter_map(Result::ok)
        .collect();

    Ok(NoteContract {
        stem,
        rel_path,
        title,
        summary,
        status,
        repo,
        host,
        network,
        ports,
        dependencies,
        dependents,
        verifications,
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GraphNode {
    pub stem: String,
    pub rel_path: String,
    pub depth: usize,
    pub children: Vec<GraphNode>,
}

pub fn traverse_graph(
    vault: &Path,
    target: &str,
    max_depth: usize,
    direction: &str,
) -> Result<serde_json::Value> {
    let mut con = open_cache_db(vault)?;
    let _ = sync_vault_index(vault, &mut con)?;

    let clean_target = target.trim().trim_end_matches(".md");
    let stem = Path::new(clean_target).file_stem().and_then(|s| s.to_str()).unwrap_or(clean_target);

    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    queue.push_back((stem.to_string(), 0));
    visited.insert(stem.to_string());

    let mut results = Vec::new();

    while let Some((curr, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }

        let mut neighbors = Vec::new();

        if direction == "down" || direction == "both" {
            let exact_rel = format!("{}.md", curr);
            let like_pattern = format!("%/{}%", curr);
            let mut stmt = con.prepare("SELECT target_stem FROM relations WHERE source_rel = ?1 OR source_rel LIKE ?2")?;
            let rows = stmt.query_map(params![exact_rel, like_pattern], |r| r.get::<_, String>(0))?;
            for n in rows.flatten() {
                neighbors.push((n, "downstream"));
            }
        }

        if direction == "up" || direction == "both" {
            let mut stmt = con.prepare("SELECT source_rel FROM relations WHERE target_stem = ?1")?;
            let rows = stmt.query_map(params![curr], |r| r.get::<_, String>(0))?;
            for n in rows.flatten() {
                let n_stem = Path::new(&n).file_stem().and_then(|s| s.to_str()).unwrap_or(&n).to_string();
                neighbors.push((n_stem, "upstream"));
            }
        }

        for (next_stem, rel_type) in neighbors {
            if visited.insert(next_stem.clone()) {
                results.push(serde_json::json!({
                    "from": curr,
                    "to": next_stem,
                    "relation": rel_type,
                    "depth": depth + 1
                }));
                queue.push_back((next_stem, depth + 1));
            }
        }
    }

    Ok(serde_json::json!({
        "target": stem,
        "max_depth": max_depth,
        "direction": direction,
        "edges": results
    }))
}
