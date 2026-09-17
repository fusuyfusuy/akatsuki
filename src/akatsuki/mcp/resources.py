"""Model Context Protocol (MCP) resource definitions and handlers for Akatsuki."""

import datetime
import json

from akatsuki.index import fts_db_context, sync_fts_index
from akatsuki.storage import get_vault, resolve_note_file

MCP_RESOURCES = [
    {
        "uri": "akatsuki://services",
        "name": "Live Services Catalog",
        "description": "Active containerized services, port allocations, and ingress routing.",
        "mimeType": "application/json",
    },
    {
        "uri": "akatsuki://projects",
        "name": "Projects Inventory",
        "description": "Active production projects, tools, and technical architectures.",
        "mimeType": "application/json",
    },
    {
        "uri": "akatsuki://operator",
        "name": "Operator Profile & Machine Specs",
        "description": "Hardware telemetry, communication standards, and system topology.",
        "mimeType": "text/markdown",
    },
    {
        "uri": "akatsuki://daily",
        "name": "Today's Operational Log",
        "description": "Daily activity ledger, focus horizons, and session notes for today.",
        "mimeType": "text/markdown",
    },
    {
        "uri": "akatsuki://invariants",
        "name": "Global Architectural Invariants",
        "description": "System-wide architectural contracts, non-negotiable boundaries, and protocols.",
        "mimeType": "text/markdown",
    },
    {
        "uri": "akatsuki://index",
        "name": "Master Vault Catalog",
        "description": "Root index and map of contents across all knowledge domains.",
        "mimeType": "text/markdown",
    },
]


def handle_mcp_resource_read(uri: str) -> tuple[str, bool]:
    vault = get_vault()
    if uri == "akatsuki://services":
        with fts_db_context(vault) as con:
            sync_fts_index(vault, con)
            cur = con.execute("SELECT name, container_prefix, ports, replicas, role, host, network FROM services")
            rows = [dict(r) for r in cur.fetchall()]
        if rows:
            return json.dumps(rows, indent=2, default=str), False
        cat = vault / "40-Systems" / "Services-Catalog.md"
        if cat.exists():
            return cat.read_text(encoding="utf-8"), False
        return "Services-Catalog.md not found in vault.", True

    elif uri == "akatsuki://projects":
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

    elif uri == "akatsuki://operator":
        op = vault / "OPERATOR.md"
        if op.exists():
            return op.read_text(encoding="utf-8"), False
        return "OPERATOR.md not found in vault.", True

    elif uri == "akatsuki://daily":
        today_str = datetime.datetime.now().astimezone().strftime("%Y-%m-%d")
        daily = vault / "01-Daily" / f"{today_str}.md"
        if daily.exists():
            return daily.read_text(encoding="utf-8"), False
        return f"Daily note for {today_str} does not exist.", False

    elif uri == "akatsuki://invariants":
        agents_md = vault / "AGENTS.md"
        if agents_md.exists():
            return agents_md.read_text(encoding="utf-8"), False
        return "AGENTS.md not found in vault.", True

    elif uri == "akatsuki://index":
        index_md = vault / "INDEX.md"
        if index_md.exists():
            return index_md.read_text(encoding="utf-8"), False
        return "INDEX.md not found in vault.", True

    elif uri.startswith("akatsuki://"):
        stem_or_path = uri.replace("akatsuki://", "")
        note_file = resolve_note_file(vault, stem_or_path)
        if note_file and note_file.exists():
            return note_file.read_text(encoding="utf-8"), False
        return f"Resource '{uri}' not found in akatsuki vault.", True
    return f"Unsupported resource URI scheme: '{uri}'", True
