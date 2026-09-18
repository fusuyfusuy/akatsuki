"""Model Context Protocol (MCP) tool schemas and call dispatchers for Akatsuki."""

import datetime
import json
import re

from akatsuki.graph import calculate_blast_radius, traverse_graph
from akatsuki.index import execute_sql_query, fts_db_context, sync_fts_index
from akatsuki.markdown import apply_token_budget, slice_markdown_section
from akatsuki.mutations import (
    append_section_to_note,
    append_work_log,
    list_notes_in_vault,
    replace_section_in_note,
    set_note_property,
    write_note,
)
from akatsuki.search import extract_note_contract, get_keypath, search_vault
from akatsuki.storage import get_vault, resolve_note_file
from akatsuki.verify import lint_vault, reconcile_vault, run_verification_tests, verify_links

MCP_TOOLS = [
    {
        "name": "akatsuki_search",
        "description": "Search notes, architecture specifications, and infrastructure configs using hybrid BM25 + dense semantic vector fusion, with optional 1-hop graph relations.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search terms, topic, natural language query, or exact phrase in quotes.",
                },
                "mode": {
                    "type": "string",
                    "enum": ["hybrid", "bm25", "vector"],
                    "description": "Retrieval mode: 'hybrid' (Okapi BM25 + dense semantic vectors via RRF, default), 'bm25' (pure lexical), or 'vector' (dense embeddings).",
                },
                "domain": {
                    "type": "string",
                    "description": "Optional domain directory filter (e.g. '40-Systems', '20-Projects', '30-Agents', '01-Daily').",
                },
                "limit": {
                    "type": "integer",
                    "default": 10,
                    "description": "Maximum number of search matches to return (default: 10).",
                },
                "with_graph": {
                    "type": "boolean",
                    "default": False,
                    "description": "If true, attach immediate upstream dependents, downstream dependencies, and container/port allocations directly to results.",
                },
            },
            "required": ["query"],
        },
    },
    {
        "name": "akatsuki_read",
        "description": "Read a note, specification, or ADR from akatsuki. Supports surgical heading extraction and token budget packing.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "note": {
                    "type": "string",
                    "description": "Note title, stem, or relative path (e.g. 'Traefik-Ingress', '20-Projects/web').",
                },
                "section": {
                    "type": "string",
                    "description": "Optional section heading to extract (e.g. 'Invariants', 'Network Topology').",
                },
                "budget": {
                    "type": "integer",
                    "description": "Optional token budget limit (e.g. 300). Truncates content cleanly if exceeded.",
                },
            },
            "required": ["note"],
        },
    },
    {
        "name": "akatsuki_contract",
        "description": "Extract machine boundary contract from a note, including declared ports, networks, relations, invariants, and live verifications.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "note": {
                    "type": "string",
                    "description": "Target note title, stem, or relative path.",
                },
                "json": {
                    "type": "boolean",
                    "default": False,
                    "description": "If true, return contract as structured JSON payload instead of YAML.",
                },
            },
            "required": ["note"],
        },
    },
    {
        "name": "akatsuki_get",
        "description": "O(1) exact property getter across services, entities, and note frontmatter keypaths.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "Keypath to extract (e.g. 'services.api.ports', 'entities.web.status', 'Traefik.tags').",
                },
            },
            "required": ["key"],
        },
    },
    {
        "name": "akatsuki_query",
        "description": "Execute read-only SQL queries directly against the internal SQLite index (notes_fts, entities, services, relations, invariants, verifications).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "sql": {
                    "type": "string",
                    "description": "Read-only SQL query string (SELECT ...).",
                },
            },
            "required": ["sql"],
        },
    },
    {
        "name": "akatsuki_blast",
        "description": "Calculate upstream callers, downstream dependencies, and container/port boundary sinks for architectural blast radius.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "description": "Component, service, or system name to analyze.",
                },
                "format": {
                    "type": "string",
                    "enum": ["text", "json"],
                    "default": "text",
                    "description": "Output format: 'text' (markdown report) or 'json' (structured payload).",
                },
            },
            "required": ["target"],
        },
    },
    {
        "name": "akatsuki_map",
        "description": "Recursively map knowledge graph around a target note up to N hops, detecting cycles and boundary port allocations.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "description": "Root note or component to map.",
                },
                "depth": {
                    "type": "integer",
                    "default": 2,
                    "description": "Traversal depth in hops (1-5, default: 2).",
                },
                "direction": {
                    "type": "string",
                    "enum": ["both", "down", "up"],
                    "default": "both",
                    "description": "Traversal direction: 'both', 'down' (dependencies), or 'up' (dependents).",
                },
                "format": {
                    "type": "string",
                    "enum": ["text", "json"],
                    "default": "text",
                    "description": "Output format: 'text' (ASCII/Markdown tree) or 'json' (graph payload).",
                },
            },
            "required": ["target"],
        },
    },
    {
        "name": "akatsuki_test",
        "description": "Execute machine-verifiable invariant assertion blocks (```bash:verify) embedded in notes.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "note": {
                    "type": "string",
                    "description": "Optional note title or stem to filter tests. If omitted, runs all tests.",
                },
                "target": {
                    "type": "string",
                    "description": "Alias for 'note'. Optional note title or stem to filter tests.",
                },
                "dry_run": {
                    "type": "boolean",
                    "default": False,
                    "description": "If true, inspects assertion commands without executing them in a subshell.",
                },
            },
        },
    },
    {
        "name": "akatsuki_set",
        "description": "Surgically update a frontmatter key-value property without corrupting note body.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "note": {
                    "type": "string",
                    "description": "Note title, stem, or relative path.",
                },
                "key": {
                    "type": "string",
                    "description": "Dot-separated keypath (e.g. 'status', 'ports.0').",
                },
                "value": {
                    "type": "string",
                    "description": "New value as string, number, boolean, or JSON-encoded object.",
                },
            },
            "required": ["note", "key", "value"],
        },
    },
    {
        "name": "akatsuki_lint",
        "description": "Validate that all vault notes conform to strict machine schemas and valid frontmatter YAML.",
        "inputSchema": {
            "type": "object",
            "properties": {},
        },
    },
    {
        "name": "akatsuki_append_section",
        "description": "Safely append content under a specific heading inside an existing note under kernel lock.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "note": {
                    "type": "string",
                    "description": "Note title, stem, or relative path.",
                },
                "heading": {
                    "type": "string",
                    "description": "Heading section under which content is appended.",
                },
                "content": {
                    "type": "string",
                    "description": "Markdown text or bullets to append.",
                },
            },
            "required": ["note", "heading", "content"],
        },
    },
    {
        "name": "akatsuki_replace_section",
        "description": "Surgically replace the contents of a specific markdown heading within a note under kernel lock.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "note": {
                    "type": "string",
                    "description": "Note title, stem, or relative path.",
                },
                "heading": {
                    "type": "string",
                    "description": "Heading section to replace.",
                },
                "content": {
                    "type": "string",
                    "description": "New replacement markdown content.",
                },
            },
            "required": ["note", "heading", "content"],
        },
    },
    {
        "name": "akatsuki_services",
        "description": "Read active containerized services, port allocations, and ingress routing.",
        "inputSchema": {
            "type": "object",
            "properties": {},
        },
    },
    {
        "name": "akatsuki_projects",
        "description": "Read active production projects, tools, and technical architectures.",
        "inputSchema": {
            "type": "object",
            "properties": {},
        },
    },
    {
        "name": "akatsuki_daily",
        "description": "Read today's or specified daily horizon and work ledger.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "date": {
                    "type": "string",
                    "description": "Optional YYYY-MM-DD date (defaults to today).",
                },
            },
        },
    },
    {
        "name": "akatsuki_record_log",
        "description": "Append a telegraphic caveman work log entry into today's active note under kernel lock.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "project": {
                    "type": "string",
                    "description": "Project or repo name (e.g. 'web', 'filament', 'traefik').",
                },
                "summary": {
                    "type": "string",
                    "description": "Punchy telegraphic summary (<=280 chars; syntax: '<verb> <target> -> <delta>; <evidence>').",
                },
                "device": {
                    "type": "string",
                    "description": "Optional host/machine identifier (defaults to current hostname).",
                },
            },
            "required": ["summary"],
        },
    },
    {
        "name": "akatsuki_write_note",
        "description": "Create or update a note in the vault with auto-healed YAML frontmatter and link verification.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Vault-relative path (e.g. '20-Projects/my-app.md', '50-Configs/caddy.conf').",
                },
                "content": {
                    "type": "string",
                    "description": "Full text content of the note or config.",
                },
                "overwrite": {
                    "type": "boolean",
                    "default": False,
                    "description": "If true, overwrite file if it already exists.",
                },
                "raw": {
                    "type": "boolean",
                    "default": False,
                    "description": "If true, write raw file without enforcing markdown extension or frontmatter.",
                },
            },
            "required": ["path", "content"],
        },
    },
    {
        "name": "akatsuki_verify",
        "description": "Verify that all wikilinks resolve cleanly with zero broken links or orphan notes across the vault.",
        "inputSchema": {
            "type": "object",
            "properties": {},
        },
    },
    {
        "name": "akatsuki_list_notes",
        "description": "List notes in vault with metadata and domain filtering.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "domain": {
                    "type": "string",
                    "description": "Optional domain filter (e.g. '20-Projects', '40-Systems').",
                },
            },
        },
    },
    {
        "name": "akatsuki_reconcile",
        "description": "Auto-reconcile unindexed notes and strict YAML quoting across the vault.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "dry_run": {
                    "type": "boolean",
                    "default": False,
                    "description": "If true, simulate reconciliation without writing changes to disk.",
                },
                "with_vectors": {
                    "type": "boolean",
                    "default": False,
                    "description": "If true, also synchronize the vector index.",
                },
            },
        },
    },
]


def handle_mcp_call(name: str, args: dict) -> tuple[str, bool]:
    vault = get_vault()
    if name == "akatsuki_search":
        query = args.get("query", "")
        domain = args.get("domain")
        limit = args.get("limit", 10)
        with_graph = bool(args.get("with_graph", False))
        mode = args.get("mode", "hybrid")
        results = search_vault(
            vault,
            query,
            domain=domain,
            limit=limit,
            with_graph=with_graph,
            mode=mode,
        )
        if not results:
            return f"No notes found matching '{query}'.", False
        out = [f"Found {len(results)} matching note(s) [Mode: {mode}]:"]
        for r in results:
            out.append(f"\n- **{r['title']}** (`{r['rel_path']}`) [Score: {r['score']}]: {r['summary']}")
            if r.get("breadcrumb"):
                out.append(f"    Section: {r['breadcrumb']}")
            if r.get("snippet"):
                out.append(f"    Excerpt: {r['snippet']}")
            if r.get("graph"):
                g = r["graph"]
                g_parts = []
                if g.get("upstream"):
                    g_parts.append(f"Upstream: {', '.join(g['upstream'])}")
                if g.get("downstream"):
                    g_parts.append(f"Downstream: {', '.join(g['downstream'])}")
                if g.get("services"):
                    g_parts.append(f"Boundary: {', '.join(g['services'])}")
                if g_parts:
                    out.append(f"    Graph: {' | '.join(g_parts)}")
        return "\n".join(out), False

    elif name == "akatsuki_read":
        note_name = args.get("note", "")
        section = args.get("section")
        budget = args.get("budget")
        if not note_name:
            return "Error: Missing required parameter 'note'.", True
        note_file = resolve_note_file(vault, note_name)
        if not note_file:
            return f"Note '{note_name}' not found in akatsuki vault.", True
        content = note_file.read_text(encoding="utf-8")
        if section:
            sliced, toc_lines = slice_markdown_section(content, section)
            if sliced is None:
                err_msg = [
                    f"Section '{section}' not found in '{note_file.name}'.",
                    "Available sections:",
                ]
                for t in toc_lines:
                    err_msg.append(f"  - {t}")
                return "\n".join(err_msg), True
            content = sliced

        if budget:
            content = apply_token_budget(content, budget)
        return content, False

    elif name == "akatsuki_contract":
        note = args.get("note", "")
        if not note:
            return "Error: Missing parameter 'note'.", True
        as_json = bool(args.get("json", False))
        return extract_note_contract(vault, note, as_json=as_json)

    elif name == "akatsuki_get":
        key = args.get("key", "")
        if not key:
            return "Error: Missing parameter 'key'.", True
        return get_keypath(vault, key)

    elif name == "akatsuki_query":
        sql = args.get("sql", "")
        if not sql:
            return "Error: Missing parameter 'sql'.", True
        return execute_sql_query(vault, sql)

    elif name == "akatsuki_blast":
        target = args.get("target", "")
        if not target:
            return "Error: Missing parameter 'target'.", True
        fmt = args.get("format", "text")
        as_json = fmt == "json" or bool(args.get("json", False))
        return calculate_blast_radius(vault, target, as_json=as_json)

    elif name == "akatsuki_map":
        target = args.get("target", "")
        if not target:
            return "Error: Missing parameter 'target'.", True
        depth = args.get("depth", 2)
        direction = args.get("direction", "both")
        fmt = args.get("format", "text")
        as_json = fmt == "json" or bool(args.get("json", False))
        text_out, is_err, json_data = traverse_graph(vault, target, depth=depth, direction=direction)
        if as_json:
            return json.dumps(json_data, indent=2, default=str), is_err
        return text_out, is_err

    elif name == "akatsuki_test":
        note = args.get("note") or args.get("target")
        dry_run = bool(args.get("dry_run", False))
        return run_verification_tests(vault, note_filter=note, dry_run=dry_run)

    elif name == "akatsuki_set":
        note = args.get("note", "")
        key = args.get("key", "")
        val = args.get("value", "")
        if not note or not key:
            return "Error: Both 'note' and 'key' parameters are required.", True
        return set_note_property(vault, note, key, val)

    elif name == "akatsuki_lint":
        return lint_vault(vault)

    elif name == "akatsuki_append_section":
        note_name = args.get("note", "")
        heading = args.get("heading", "")
        content = args.get("content", "")
        if not note_name or not heading or not content:
            return (
                "Error: Parameters 'note', 'heading', and 'content' are all required.",
                True,
            )
        res, is_err = append_section_to_note(vault, note_name, heading, content)
        return res, is_err

    elif name == "akatsuki_replace_section":
        note_name = args.get("note", "")
        heading = args.get("heading", "")
        content = args.get("content", "")
        if not note_name or not heading or not content:
            return (
                "Error: Parameters 'note', 'heading', and 'content' are all required.",
                True,
            )
        res, is_err = replace_section_in_note(vault, note_name, heading, content)
        return res, is_err

    elif name == "akatsuki_services":
        with fts_db_context(vault) as con:
            sync_fts_index(vault, con)
            cur = con.execute("SELECT name, container_prefix, ports, replicas, role, host, network FROM services")
            rows = [dict(r) for r in cur.fetchall()]
        if rows:
            return json.dumps(rows, indent=2, default=str), False
        catalog = vault / "40-Systems" / "Services-Catalog.md"
        if catalog.exists():
            return catalog.read_text(encoding="utf-8"), False
        return "Services-Catalog.md not found in vault.", True

    elif name == "akatsuki_projects":
        with fts_db_context(vault) as con:
            sync_fts_index(vault, con)
            cur = con.execute(
                "SELECT stem, title, status, repo, host, network, summary FROM entities WHERE type = 'project'"
            )
            rows = [dict(r) for r in cur.fetchall()]
        if rows:
            return json.dumps(rows, indent=2, default=str), False
        moc = vault / "20-Projects" / "Projects-MOC.md"
        if not moc.exists():
            moc = vault / "INDEX.md"
        return moc.read_text(encoding="utf-8"), False

    elif name == "akatsuki_daily":
        d = args.get("date")
        if d:
            d = str(d).strip()
            if not re.match(r"^\d{4}-\d{2}-\d{2}$", d):
                return f"Error: Invalid date format '{d}'. Expected YYYY-MM-DD.", True
        else:
            d = datetime.datetime.now().astimezone().strftime("%Y-%m-%d")
        daily = vault / "01-Daily" / f"{d}.md"
        if daily.exists():
            return daily.read_text(encoding="utf-8"), False
        return f"Daily note for {d} does not exist.", False

    elif name == "akatsuki_record_log":
        project = args.get("project", "")
        summary = args.get("summary", "")
        device = args.get("device")
        if not summary:
            return "Error: Missing required parameter 'summary'.", True
        res = append_work_log(vault, project, summary, device=device)
        return res, False

    elif name == "akatsuki_write_note":
        path = args.get("path", "")
        content = args.get("content", "")
        overwrite = args.get("overwrite", False)
        raw = args.get("raw", False)
        if isinstance(overwrite, str):
            overwrite = overwrite.lower() in ("true", "1", "yes")
        if isinstance(raw, str):
            raw = raw.lower() in ("true", "1", "yes")
        if not path or not content:
            return "Error: Both 'path' and 'content' parameters are required.", True
        res, is_err = write_note(vault, path, content, overwrite=overwrite, raw=raw)
        return res, is_err

    elif name == "akatsuki_verify":
        ok, broken = verify_links(vault)
        if not ok:
            out = [f"FAILED: {len(broken)} link/graph issue(s) found in akatsuki:"]
            for src, issue in broken[:15]:
                out.append(f"- In '{src}': {issue}")
            return "\n".join(out), True
        return "PASSED: All wikilinks and markdown links in akatsuki resolve cleanly (zero orphans).", False

    elif name == "akatsuki_list_notes":
        domain = args.get("domain")
        notes = list_notes_in_vault(vault, domain=domain)
        if not notes:
            return f"No notes found in domain '{domain}'.", False
        out = [f"Notes in akatsuki ({len(notes)} total):"]
        for n in notes:
            out.append(f"- **{n['title']}** (`{n['rel_path']}`) [{n['type']}]")
            if n["summary"]:
                out.append(f"  Summary: {n['summary']}")
        return "\n".join(out), False

    elif name == "akatsuki_reconcile":
        dry_run = args.get("dry_run", False)
        if isinstance(dry_run, str):
            dry_run = dry_run.lower() in ("true", "1", "yes")
        with_vectors = args.get("with_vectors", False)
        if isinstance(with_vectors, str):
            with_vectors = with_vectors.lower() in ("true", "1", "yes")
        res, is_err = reconcile_vault(vault, dry_run=dry_run, with_vectors=with_vectors)
        return res, is_err

    return f"Unknown tool: {name}", True
