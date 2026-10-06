//! Knowledge graph, blast radius calculation, map traversal, and contract extraction.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::index::open_synced_db;
use crate::storage::{parse_frontmatter, resolve_note_file};

/// Escapes SQL LIKE special wildcard characters ('%', '_', '\').
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Upstream dependents, downstream dependencies, and boundary sinks for a target.
#[derive(Debug, Serialize, Deserialize)]
pub struct BlastRadius {
    pub target: String,
    pub upstream: Vec<UpstreamRef>,
    pub downstream: Vec<DownstreamRef>,
    pub boundary_sinks: Vec<ServiceItem>,
}

/// A note that depends on the target, matched through `relations.target_stem`.
#[derive(Debug, Serialize, Deserialize)]
pub struct UpstreamRef {
    pub source_rel: String,
    pub relation_type: String,
}

/// A dependency declared by the target, matched through `relations.source_rel`.
#[derive(Debug, Serialize, Deserialize)]
pub struct DownstreamRef {
    pub target_stem: String,
    pub relation_type: String,
}

/// A container/port allocation the indexer projected for a note.
#[derive(Debug, Serialize, Deserialize)]
pub struct ServiceItem {
    pub name: String,
    pub container_prefix: String,
    pub ports: String,
    pub host: String,
    pub network: String,
    pub rel_path: String,
}

/// Normalizes a user-supplied target into the note stem used by the index.
fn target_stem(target: &str) -> String {
    let trimmed = target.trim().trim_end_matches(".md");
    Path::new(trimmed)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(trimmed)
        .to_string()
}

/// Stem of a vault-relative path, mirroring `Path.stem` on the note reference.
fn path_stem(rel_path: &str) -> String {
    Path::new(rel_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(rel_path)
        .to_string()
}

/// Vault-relative path of a stem, falling back to `{stem}.md` for unindexed notes.
fn resolve_stem_rel_path(con: &Connection, stem: &str) -> Result<String> {
    let rel: Option<String> = con
        .query_row(
            "SELECT rel_path FROM entities WHERE stem = ?1 LIMIT 1",
            params![stem],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    Ok(rel
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| format!("{}.md", stem)))
}

/// Computes the blast radius of `stem` against an already-reconciled index.
fn blast_radius_with(con: &Connection, stem: &str) -> Result<BlastRadius> {
    let mut stmt_up =
        con.prepare("SELECT source_rel, relation_type FROM relations WHERE target_stem = ?1")?;
    let upstream: Vec<UpstreamRef> = stmt_up
        .query_map(params![stem], |r| {
            Ok(UpstreamRef {
                source_rel: r.get(0)?,
                relation_type: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let escaped_stem = escape_like(stem);
    let exact_rel = format!("{}.md", stem);
    let nested_rel = format!("%/{}.md", escaped_stem);
    let descendant_rel = format!("%/{}/%", escaped_stem);
    let mut stmt_down = con.prepare(
        "SELECT target_stem, relation_type FROM relations \
         WHERE source_rel = ?1 OR source_rel LIKE ?2 ESCAPE '\\' OR source_rel LIKE ?3 ESCAPE '\\'",
    )?;
    let downstream: Vec<DownstreamRef> = stmt_down
        .query_map(params![exact_rel, nested_rel, descendant_rel], |r| {
            Ok(DownstreamRef {
                target_stem: r.get(0)?,
                relation_type: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let sink_rel = format!("%/{}.md", escaped_stem);
    let mut stmt_svc = con.prepare(
        "SELECT name, container_prefix, ports, host, network, rel_path FROM services \
         WHERE name = ?1 OR container_prefix = ?1 OR rel_path LIKE ?2 ESCAPE '\\'",
    )?;
    let boundary_sinks: Vec<ServiceItem> = stmt_svc
        .query_map(params![stem, sink_rel], |r| {
            Ok(ServiceItem {
                name: r.get(0)?,
                container_prefix: r.get(1)?,
                ports: r.get(2)?,
                host: r.get(3)?,
                network: r.get(4)?,
                rel_path: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(BlastRadius {
        target: stem.to_string(),
        upstream,
        downstream,
        boundary_sinks,
    })
}

/// Calculates upstream dependents, downstream dependencies, and boundary sinks.
pub fn calculate_blast_radius(vault: &Path, target: &str) -> Result<BlastRadius> {
    let con = open_synced_db(vault)?;
    blast_radius_with(&con, &target_stem(target))
}

/// A relation row attached to a contract's note.
#[derive(Debug, Serialize, Deserialize)]
pub struct NoteRelation {
    pub relation_type: String,
    pub target: String,
}

/// Machine boundary contract of a single note.
#[derive(Debug, Serialize, Deserialize)]
pub struct NoteContract {
    pub stem: String,
    pub rel_path: String,
    pub title: String,
    pub note_type: String,
    pub summary: String,
    pub status: Option<String>,
    pub repo: Option<String>,
    pub host: Option<String>,
    pub network: Option<String>,
    pub ports: Vec<String>,
    pub relations: Vec<NoteRelation>,
    pub invariants: Vec<String>,
    pub dependencies: Vec<String>,
    pub dependents: Vec<String>,
    pub verifications: Vec<String>,
}

/// Extracts the machine boundary contract of the note named by `note_query`.
pub fn extract_contract(vault: &Path, note_query: &str) -> Result<NoteContract> {
    let note_path = resolve_note_file(vault, note_query)
        .with_context(|| format!("Note '{}' not found in vault", note_query))?;

    let content = std::fs::read_to_string(&note_path)?;
    let (fm, _) = parse_frontmatter(&content)
        .with_context(|| format!("Note '{}' has invalid frontmatter", note_path.display()))?;

    let stem = note_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let rel_path = note_path
        .strip_prefix(vault)
        .unwrap_or(&note_path)
        .to_string_lossy()
        .to_string();

    let title = fm
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or(&stem)
        .to_string();
    let note_type = fm
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("note")
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

    let con = open_synced_db(vault)?;
    let blast = blast_radius_with(&con, &stem)?;

    let mut ports = Vec::new();
    for svc in &blast.boundary_sinks {
        if !svc.ports.is_empty() {
            ports.push(format!("{}: {}", svc.name, svc.ports));
        }
    }

    let dependencies: Vec<String> = blast
        .downstream
        .into_iter()
        .map(|d| d.target_stem)
        .collect();
    let dependents: Vec<String> = blast.upstream.into_iter().map(|u| u.source_rel).collect();

    let mut stmt_rel =
        con.prepare("SELECT relation_type, target_stem FROM relations WHERE source_rel = ?1")?;
    let relations: Vec<NoteRelation> = stmt_rel
        .query_map(params![rel_path], |r| {
            Ok(NoteRelation {
                relation_type: r.get(0)?,
                target: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut stmt_inv =
        con.prepare("SELECT invariant_text FROM invariants WHERE source_rel = ?1")?;
    let invariants: Vec<String> = stmt_inv
        .query_map(params![rel_path], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut stmt = con.prepare("SELECT command FROM verifications WHERE source_rel = ?1")?;
    let verifications: Vec<String> = stmt
        .query_map(params![rel_path], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(NoteContract {
        stem,
        rel_path,
        title,
        note_type,
        summary,
        status,
        repo,
        host,
        network,
        ports,
        relations,
        invariants,
        dependencies,
        dependents,
        verifications,
    })
}

/// One node of a rendered traversal tree.
#[derive(Debug, Serialize)]
struct TreeNode {
    stem: String,
    rel_path: String,
    rel_type: String,
    cycle: bool,
    children: Vec<TreeNode>,
}

/// A boundary sink as carried in the map payload.
#[derive(Debug, Serialize)]
struct MapSink {
    name: String,
    ports: String,
    host: String,
    network: String,
    rel_path: String,
}

/// The map JSON payload, in traversal order.
#[derive(Debug, Serialize)]
struct MapPayloadJson<'a> {
    target: &'a str,
    rel_path: String,
    depth: usize,
    direction: &'a str,
    downstream: &'a [TreeNode],
    upstream: &'a [TreeNode],
    boundary_sinks: &'a [MapSink],
}

/// Structured traversal plus its rendered ASCII tree.
#[derive(Debug)]
pub struct MapPayload {
    pub payload: serde_json::Value,
    pub text: String,
}

/// Children reachable from `stem` by declared dependencies, up to `max_depth`.
fn traverse_down(
    con: &Connection,
    stem: &str,
    current_depth: usize,
    max_depth: usize,
    ancestors: &[String],
    visited: &mut HashSet<String>,
) -> Result<Vec<TreeNode>> {
    if current_depth >= max_depth {
        return Ok(Vec::new());
    }

    let escaped_stem = escape_like(stem);
    let exact_rel = format!("{}.md", stem);
    let nested_rel = format!("%/{}.md", escaped_stem);
    let descendant_rel = format!("%/{}/%", escaped_stem);
    let mut stmt = con.prepare(
        "SELECT target_stem, relation_type FROM relations \
         WHERE source_rel = ?1 OR source_rel LIKE ?2 ESCAPE '\\' OR source_rel LIKE ?3 ESCAPE '\\'",
    )?;
    let rows: Vec<(String, String)> = stmt
        .query_map(params![exact_rel, nested_rel, descendant_rel], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut children = Vec::with_capacity(rows.len());
    for (child_stem, relation_type) in rows {
        let cycle = ancestors.iter().any(|a| a == &child_stem);
        let mut nested = Vec::new();
        if !cycle && visited.insert(child_stem.clone()) {
            let mut path = ancestors.to_vec();
            path.push(child_stem.clone());
            nested = traverse_down(con, &child_stem, current_depth + 1, max_depth, &path, visited)?;
        }
        children.push(TreeNode {
            rel_path: resolve_stem_rel_path(con, &child_stem)?,
            stem: child_stem,
            rel_type: relation_type,
            cycle,
            children: nested,
        });
    }
    Ok(children)
}

/// Dependents of `stem` that point at it through `relations.target_stem`.
fn traverse_up(
    con: &Connection,
    stem: &str,
    current_depth: usize,
    max_depth: usize,
    ancestors: &[String],
    visited: &mut HashSet<String>,
) -> Result<Vec<TreeNode>> {
    if current_depth >= max_depth {
        return Ok(Vec::new());
    }

    let mut stmt =
        con.prepare("SELECT source_rel, relation_type FROM relations WHERE target_stem = ?1")?;
    let rows: Vec<(String, String)> = stmt
        .query_map(params![stem], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut children = Vec::with_capacity(rows.len());
    for (source_rel, relation_type) in rows {
        let child_stem = path_stem(&source_rel);
        let cycle = ancestors.iter().any(|a| a == &child_stem);
        let mut nested = Vec::new();
        if !cycle && visited.insert(child_stem.clone()) {
            let mut path = ancestors.to_vec();
            path.push(child_stem.clone());
            nested = traverse_up(con, &child_stem, current_depth + 1, max_depth, &path, visited)?;
        }
        children.push(TreeNode {
            stem: child_stem,
            rel_path: source_rel,
            rel_type: relation_type,
            cycle,
            children: nested,
        });
    }
    Ok(children)
}

/// Collects every stem appearing in a tree, regardless of cycle marking.
fn collect_stems(nodes: &[TreeNode], stems: &mut BTreeSet<String>) {
    for node in nodes {
        stems.insert(node.stem.clone());
        collect_stems(&node.children, stems);
    }
}

/// Renders tree nodes recursively into ASCII/Markdown lines with cycle markers.
fn render_tree_lines(nodes: &[TreeNode], prefix: &str, out: &mut Vec<String>) {
    for (i, node) in nodes.iter().enumerate() {
        let is_last = i + 1 == nodes.len();
        let connector = if is_last { "└── " } else { "├── " };
        let rel_type = if node.rel_type.is_empty() {
            String::new()
        } else {
            format!("[{}] ", node.rel_type)
        };
        let cycle = if node.cycle { " ↺ (cycle)" } else { "" };
        let mut display = format!("**{}**", node.stem);
        if !node.rel_path.is_empty() && node.rel_path != format!("{}.md", node.stem) {
            display.push_str(&format!(" (`{}`)", node.rel_path));
        }
        out.push(format!(
            "{}{}{}{}{}",
            prefix, connector, rel_type, display, cycle
        ));

        if !node.children.is_empty() {
            let child_prefix = format!("{}{}", prefix, if is_last { "    " } else { "│   " });
            render_tree_lines(&node.children, &child_prefix, out);
        }
    }
}

/// Maps the knowledge graph around `target` up to `depth` hops in `direction`.
///
/// `depth` is clamped to 1..=5 and an unknown `direction` falls back to `both`,
/// matching the legacy traversal contract.
pub fn traverse_graph(
    vault: &Path,
    target: &str,
    depth: usize,
    direction: &str,
) -> Result<MapPayload> {
    let depth = depth.clamp(1, 5);
    let direction = direction.trim().to_lowercase();
    let direction = if matches!(direction.as_str(), "both" | "down" | "up") {
        direction
    } else {
        "both".to_string()
    };

    let con = open_synced_db(vault)?;
    let stem = target_stem(target);
    let root = vec![stem.clone()];

    let mut down_visited = HashSet::new();
    down_visited.insert(stem.clone());
    let downstream = if direction == "both" || direction == "down" {
        traverse_down(&con, &stem, 0, depth, &root, &mut down_visited)?
    } else {
        Vec::new()
    };

    let mut up_visited = HashSet::new();
    up_visited.insert(stem.clone());
    let upstream = if direction == "both" || direction == "up" {
        traverse_up(&con, &stem, 0, depth, &root, &mut up_visited)?
    } else {
        Vec::new()
    };

    let mut stems: BTreeSet<String> = BTreeSet::new();
    stems.insert(stem.clone());
    collect_stems(&downstream, &mut stems);
    collect_stems(&upstream, &mut stems);

    let mut sinks: Vec<MapSink> = Vec::new();
    let mut stmt = con.prepare(
        "SELECT name, ports, host, network, rel_path FROM services \
         WHERE name = ?1 OR container_prefix = ?1 OR rel_path LIKE ?2 ESCAPE '\\'",
    )?;
    for sink_stem in &stems {
        let escaped_sink = escape_like(sink_stem);
        let rows = stmt.query_map(params![sink_stem, format!("%/{}.md", escaped_sink)], |r| {
            Ok(MapSink {
                name: r.get(0)?,
                ports: r.get(1)?,
                host: r.get(2)?,
                network: r.get(3)?,
                rel_path: r.get(4)?,
            })
        })?;
        for row in rows {
            sinks.push(row?);
        }
    }

    let payload = serde_json::to_value(MapPayloadJson {
        target: &stem,
        rel_path: resolve_stem_rel_path(&con, &stem)?,
        depth,
        direction: &direction,
        downstream: &downstream,
        upstream: &upstream,
        boundary_sinks: &sinks,
    })?;

    let mut out = vec![format!(
        "# 🗺️ Knowledge Map: `{}` (depth: {}, direction: {})\n",
        stem, depth, direction
    )];

    if direction == "both" || direction == "down" {
        out.push("## ⬇️ Downstream Dependencies (Required by Target)".to_string());
        if downstream.is_empty() {
            out.push(format!(
                "- *No downstream dependencies detected within depth {}.*",
                depth
            ));
        } else {
            out.push(format!("- **{}**", stem));
            render_tree_lines(&downstream, "  ", &mut out);
        }
        out.push(String::new());
    }

    if direction == "both" || direction == "up" {
        out.push("## ⬆️ Upstream Dependents (Affected Services / Entry Points)".to_string());
        if upstream.is_empty() {
            out.push(format!(
                "- *No upstream dependents detected within depth {}.*",
                depth
            ));
        } else {
            out.push(format!("- **{}**", stem));
            render_tree_lines(&upstream, "  ", &mut out);
        }
        out.push(String::new());
    }

    out.push("## 🔌 Boundary Sinks (Containers, Ports & Networks)".to_string());
    if sinks.is_empty() {
        out.push("- *No discrete container/port allocation mapped.*".to_string());
    } else {
        for sink in &sinks {
            let ports = if sink.ports.is_empty() {
                String::new()
            } else {
                format!(", Ports: `{}`", sink.ports)
            };
            let host = if sink.host.is_empty() {
                String::new()
            } else {
                format!(", Host: `{}`", sink.host)
            };
            let network = if sink.network.is_empty() {
                String::new()
            } else {
                format!(", Network: `{}`", sink.network)
            };
            out.push(format!(
                "- **Service `{}`** ({}){}{}{}",
                sink.name, sink.rel_path, ports, host, network
            ));
        }
    }

    Ok(MapPayload {
        payload,
        text: out.join("\n"),
    })
}
