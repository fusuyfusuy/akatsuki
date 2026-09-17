"""Knowledge graph traversal, blast radius computation, and ASCII tree rendering."""

import json
from pathlib import Path

from akatsuki.index import fts_db_context, sync_fts_index


def calculate_blast_radius(vault: Path, target: str, as_json: bool = False) -> tuple[str, bool]:
    """Calculate upstream dependents, downstream dependencies, and boundary sinks for a target."""
    with fts_db_context(vault) as con:
        sync_fts_index(vault, con)

        t = target.strip()
        if t.endswith(".md"):
            t = t[:-3]
        t_stem = Path(t).stem

        cur = con.execute(
            "SELECT source_rel, relation_type FROM relations WHERE target_stem = ?",
            (t_stem,),
        )
        upstream = cur.fetchall()

        cur = con.execute(
            "SELECT target_stem, relation_type FROM relations WHERE source_rel = ? OR source_rel LIKE ? OR source_rel LIKE ?",
            (f"{t_stem}.md", f"%/{t_stem}.md", f"%/{t_stem}/%"),
        )
        downstream = cur.fetchall()

        cur = con.execute(
            "SELECT name, ports, host, network, rel_path FROM services WHERE name = ? OR container_prefix = ? OR rel_path LIKE ?",
            (t_stem, t_stem, f"%/{t_stem}.md"),
        )
        svcs = cur.fetchall()

    if as_json:
        data = {
            "target": t_stem,
            "upstream": [{"source_rel": r["source_rel"], "relation_type": r["relation_type"]} for r in upstream],
            "downstream": [{"target_stem": r["target_stem"], "relation_type": r["relation_type"]} for r in downstream],
            "boundary_sinks": [
                {
                    "name": s["name"],
                    "ports": s["ports"],
                    "host": s["host"],
                    "network": s["network"],
                    "rel_path": s["rel_path"],
                }
                for s in svcs
            ],
        }
        return json.dumps(data, indent=2, default=str), False

    out = [f"# 💥 Architectural Blast Radius: `{t_stem}`\n"]
    out.append("## ⬆️ Upstream Dependents (Affected Services / Entry Points)")
    if upstream:
        for r in upstream:
            out.append(f"- **`{r['source_rel']}`** (relation: `{r['relation_type']}`)")
    else:
        out.append("- *None detected in knowledge graph.*")

    out.append("\n## ⬇️ Downstream Dependencies (Required by Target)")
    if downstream:
        for r in downstream:
            out.append(f"- **`{r['target_stem']}`** (relation: `{r['relation_type']}`)")
    else:
        out.append("- *None detected in knowledge graph.*")

    out.append("\n## 🔌 Boundary Sinks (Containers, Ports & Networks)")
    if svcs:
        for s in svcs:
            out.append(
                f"- **Service `{s['name']}`**: Ports: `{s['ports']}`, Host: `{s['host']}`, Network: `{s['network']}`"
            )
    else:
        out.append("- *No discrete container/port allocation mapped.*")

    return "\n".join(out), False


def _render_tree_lines(nodes: list[dict], prefix: str = "") -> list[str]:
    """Render tree nodes recursively into ASCII/Markdown tree lines with cycle detection."""
    lines = []
    for i, node in enumerate(nodes):
        is_last = i == len(nodes) - 1
        connector = "└── " if is_last else "├── "
        rel_str = f"[{node.get('rel_type')}] " if node.get("rel_type") else ""
        cycle_str = " ↺ (cycle)" if node.get("cycle") else ""
        node_display = f"**{node['stem']}**"
        if node.get("rel_path") and node["rel_path"] != f"{node['stem']}.md":
            node_display += f" (`{node['rel_path']}`)"
        lines.append(f"{prefix}{connector}{rel_str}{node_display}{cycle_str}")

        children = node.get("children", [])
        if children:
            child_prefix = prefix + ("    " if is_last else "│   ")
            lines.extend(_render_tree_lines(children, child_prefix))
    return lines


def traverse_graph(vault: Path, target: str, depth: int = 2, direction: str = "both") -> tuple[str, bool, dict]:
    """Recursively map and traverse the knowledge graph around target up to N hops."""
    t = target.strip()
    if t.endswith(".md"):
        t = t[:-3]
    t_stem = Path(t).stem

    try:
        depth = int(depth)
    except (ValueError, TypeError):
        depth = 2
    depth = max(1, min(depth, 5))

    direction = direction.lower().strip() if direction else "both"
    if direction not in ("both", "down", "up"):
        direction = "both"

    with fts_db_context(vault) as con:
        sync_fts_index(vault, con)

        def _resolve_stem_rel_path(stem: str) -> str:
            cur = con.execute("SELECT rel_path FROM entities WHERE stem = ? LIMIT 1", (stem,))
            row = cur.fetchone()
            if row and row["rel_path"]:
                return row["rel_path"]
            return f"{stem}.md"

        def _traverse_down(current_stem: str, current_depth: int, ancestors: set[str]) -> list[dict]:
            if current_depth >= depth:
                return []
            cur = con.execute(
                "SELECT target_stem, relation_type FROM relations WHERE source_rel = ? OR source_rel LIKE ? OR source_rel LIKE ?",
                (f"{current_stem}.md", f"%/{current_stem}.md", f"%/{current_stem}/%"),
            )
            children = []
            for row in cur.fetchall():
                target_stem = row["target_stem"]
                rel_type = row["relation_type"]
                is_cycle = target_stem in ancestors
                child_node = {
                    "stem": target_stem,
                    "rel_path": _resolve_stem_rel_path(target_stem),
                    "rel_type": rel_type,
                    "cycle": is_cycle,
                    "children": [],
                }
                if not is_cycle:
                    child_node["children"] = _traverse_down(target_stem, current_depth + 1, ancestors | {target_stem})
                children.append(child_node)
            return children

        def _traverse_up(current_stem: str, current_depth: int, ancestors: set[str]) -> list[dict]:
            if current_depth >= depth:
                return []
            cur = con.execute(
                "SELECT source_rel, relation_type FROM relations WHERE target_stem = ?",
                (current_stem,),
            )
            children = []
            for row in cur.fetchall():
                src_rel = row["source_rel"]
                src_stem = Path(src_rel).stem
                rel_type = row["relation_type"]
                is_cycle = src_stem in ancestors
                child_node = {
                    "stem": src_stem,
                    "rel_path": src_rel,
                    "rel_type": rel_type,
                    "cycle": is_cycle,
                    "children": [],
                }
                if not is_cycle:
                    child_node["children"] = _traverse_up(src_stem, current_depth + 1, ancestors | {src_stem})
                children.append(child_node)
            return children

        downstream_tree = []
        if direction in ("both", "down"):
            downstream_tree = _traverse_down(t_stem, 0, {t_stem})

        upstream_tree = []
        if direction in ("both", "up"):
            upstream_tree = _traverse_up(t_stem, 0, {t_stem})

        all_stems = {t_stem}
        for c in downstream_tree:
            all_stems.add(c["stem"])
        for c in upstream_tree:
            all_stems.add(c["stem"])

        sinks = []
        for s in sorted(all_stems):
            cur = con.execute(
                "SELECT name, ports, host, network, rel_path FROM services WHERE name = ? OR container_prefix = ? OR rel_path LIKE ?",
                (s, s, f"%/{s}.md"),
            )
            for row in cur.fetchall():
                sinks.append(dict(row))

        json_payload = {
            "target": t_stem,
            "rel_path": _resolve_stem_rel_path(t_stem),
            "depth": depth,
            "direction": direction,
            "downstream": downstream_tree,
            "upstream": upstream_tree,
            "boundary_sinks": sinks,
        }

    out = [f"# 🗺️ Knowledge Map: `{t_stem}` (depth: {depth}, direction: {direction})\n"]

    if direction in ("both", "down"):
        out.append("## ⬇️ Downstream Dependencies (Required by Target)")
        if downstream_tree:
            out.append(f"- **{t_stem}**")
            out.extend(_render_tree_lines(downstream_tree, prefix="  "))
        else:
            out.append(f"- *No downstream dependencies detected within depth {depth}.*")
        out.append("")

    if direction in ("both", "up"):
        out.append("## ⬆️ Upstream Dependents (Affected Services / Entry Points)")
        if upstream_tree:
            out.append(f"- **{t_stem}**")
            out.extend(_render_tree_lines(upstream_tree, prefix="  "))
        else:
            out.append(f"- *No upstream dependents detected within depth {depth}.*")
        out.append("")

    out.append("## 🔌 Boundary Sinks (Containers, Ports & Networks)")
    if sinks:
        for sk in sinks:
            ports_str = f", Ports: `{sk['ports']}`" if sk.get("ports") else ""
            host_str = f", Host: `{sk['host']}`" if sk.get("host") else ""
            net_str = f", Network: `{sk['network']}`" if sk.get("network") else ""
            out.append(f"- **Service `{sk['name']}`** ({sk.get('rel_path', '')}){ports_str}{host_str}{net_str}")
    else:
        out.append("- *No discrete container/port allocation mapped.*")

    return "\n".join(out), False, json_payload
