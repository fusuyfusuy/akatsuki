"""SQLite index and relational store for Akatsuki."""

import json
import re
import sqlite3
from contextlib import contextmanager
from pathlib import Path

from akatsuki.markdown import slice_markdown_section
from akatsuki.storage import get_machine_id, parse_frontmatter


def get_fts_db(vault: Path) -> sqlite3.Connection:
    """Connect to vault SQLite index, initializing tables if needed."""
    cache_dir = vault / ".akatsuki"
    cache_dir.mkdir(parents=True, exist_ok=True)
    db_path = cache_dir / "index.db"
    con = sqlite3.connect(str(db_path), timeout=30.0)
    con.row_factory = sqlite3.Row
    con.execute("PRAGMA journal_mode=WAL;")

    con.execute("CREATE TABLE IF NOT EXISTS schema_meta (key TEXT PRIMARY KEY, val TEXT);")
    cur = con.execute("SELECT val FROM schema_meta WHERE key = 'version';")
    row = cur.fetchone()
    current_ver = row["val"] if row else None

    if current_ver != "5":
        con.execute("DROP TABLE IF EXISTS notes_fts;")
        con.execute("DROP TABLE IF EXISTS file_meta;")
        con.execute("DROP TABLE IF EXISTS entities;")
        con.execute("DROP TABLE IF EXISTS services;")
        con.execute("DROP TABLE IF EXISTS relations;")
        con.execute("DROP TABLE IF EXISTS invariants;")
        con.execute("DROP TABLE IF EXISTS verifications;")

        con.execute("CREATE TABLE file_meta (rel_path TEXT PRIMARY KEY, mtime REAL NOT NULL, size INTEGER NOT NULL);")
        con.execute("""
            CREATE VIRTUAL TABLE notes_fts USING fts5(
                rel_path UNINDEXED,
                stem UNINDEXED,
                domain UNINDEXED,
                title,
                tags,
                summary,
                body,
                tokenize = 'unicode61'
            );
        """)
        con.execute("""
            CREATE TABLE entities (
                rel_path TEXT PRIMARY KEY,
                stem TEXT NOT NULL,
                domain TEXT NOT NULL,
                title TEXT NOT NULL,
                type TEXT NOT NULL,
                status TEXT,
                repo TEXT,
                host TEXT,
                network TEXT,
                summary TEXT,
                updated TEXT,
                updated_by TEXT,
                metadata_json TEXT NOT NULL
            );
        """)
        con.execute("CREATE INDEX IF NOT EXISTS idx_entities_stem ON entities(stem);")
        con.execute("CREATE INDEX IF NOT EXISTS idx_entities_type ON entities(type);")
        con.execute("""
            CREATE TABLE services (
                name TEXT PRIMARY KEY,
                container_prefix TEXT,
                ports TEXT,
                replicas TEXT,
                role TEXT,
                host TEXT,
                network TEXT,
                rel_path TEXT
            );
        """)
        con.execute("""
            CREATE TABLE relations (
                source_rel TEXT NOT NULL,
                target_stem TEXT NOT NULL,
                relation_type TEXT NOT NULL,
                PRIMARY KEY (source_rel, target_stem, relation_type)
            );
        """)
        con.execute("CREATE INDEX IF NOT EXISTS idx_relations_target ON relations(target_stem);")
        con.execute("""
            CREATE TABLE invariants (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source_rel TEXT NOT NULL,
                rule TEXT NOT NULL
            );
        """)
        con.execute("""
            CREATE TABLE verifications (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source_rel TEXT NOT NULL,
                command TEXT NOT NULL
            );
        """)
        con.execute("INSERT OR REPLACE INTO schema_meta(key, val) VALUES ('version', '5');")
        con.commit()

    return con


@contextmanager
def fts_db_context(vault: Path):
    """Context manager for SQLite FTS connection, ensuring safe closure."""
    con = get_fts_db(vault)
    try:
        yield con
    finally:
        con.close()


def sync_fts_index(vault: Path, con: sqlite3.Connection) -> None:
    """Incrementally synchronize SQLite FTS5 index and relational metadata."""
    cur = con.execute("SELECT rel_path, mtime, size FROM file_meta")
    indexed = {row["rel_path"]: (row["mtime"], row["size"]) for row in cur.fetchall()}

    current_files = {}
    for f in vault.glob("**/*.md"):
        rel = str(f.relative_to(vault))
        if rel.startswith("_templates") or rel.startswith(".") or "/." in rel:
            continue
        try:
            stat = f.stat()
            current_files[rel] = (f, stat.st_mtime, stat.st_size)
        except Exception:
            continue

    # Prune deleted files
    deleted_paths = set(indexed.keys()) - set(current_files.keys())
    for del_path in deleted_paths:
        con.execute("DELETE FROM notes_fts WHERE rel_path = ?", (del_path,))
        con.execute("DELETE FROM file_meta WHERE rel_path = ?", (del_path,))
        con.execute("DELETE FROM entities WHERE rel_path = ?", (del_path,))
        con.execute("DELETE FROM services WHERE rel_path = ?", (del_path,))
        con.execute("DELETE FROM relations WHERE source_rel = ?", (del_path,))
        con.execute("DELETE FROM invariants WHERE source_rel = ?", (del_path,))
        con.execute("DELETE FROM verifications WHERE source_rel = ?", (del_path,))

    # Update new or modified files
    for rel, (f, mtime, size) in current_files.items():
        prev = indexed.get(rel)
        if prev is not None and prev[0] == mtime and prev[1] == size:
            continue

        try:
            text = f.read_text(encoding="utf-8")
        except Exception:
            continue

        fm, body = parse_frontmatter(text)
        title = str(fm.get("title") or f.stem)
        summary = str(fm.get("summary") or "")
        tags = fm.get("tags") or ""
        if isinstance(tags, list):
            tags = " ".join(str(t) for t in tags)

        domain = rel.split("/")[0] if "/" in rel else ""
        note_type = str(fm.get("type") or "note")
        status = str(fm.get("status")) if fm.get("status") else None
        repo = str(fm.get("repo")) if fm.get("repo") else None
        host = str(fm.get("host")) if fm.get("host") else None
        network = str(fm.get("network")) if fm.get("network") else None
        updated = str(fm.get("updated")) if fm.get("updated") else None
        updated_by = str(fm.get("updated_by")) if fm.get("updated_by") else None

        # Clean old records for rel
        con.execute("DELETE FROM notes_fts WHERE rel_path = ?", (rel,))
        con.execute("DELETE FROM entities WHERE rel_path = ?", (rel,))
        con.execute("DELETE FROM services WHERE rel_path = ?", (rel,))
        con.execute("DELETE FROM relations WHERE source_rel = ?", (rel,))
        con.execute("DELETE FROM invariants WHERE source_rel = ?", (rel,))
        con.execute("DELETE FROM verifications WHERE source_rel = ?", (rel,))

        # 1. Index FTS
        con.execute(
            "INSERT INTO notes_fts(rel_path, stem, domain, title, tags, summary, body) VALUES (?, ?, ?, ?, ?, ?, ?)",
            (rel, f.stem, domain, title, str(tags), summary, body),
        )
        con.execute(
            "INSERT OR REPLACE INTO file_meta(rel_path, mtime, size) VALUES (?, ?, ?)",
            (rel, mtime, size),
        )

        # 2. Index Entity
        con.execute(
            """INSERT INTO entities(rel_path, stem, domain, title, type, status, repo, host, network, summary, updated, updated_by, metadata_json)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
            (
                rel,
                f.stem,
                domain,
                title,
                note_type,
                status,
                repo,
                host,
                network,
                summary,
                updated,
                updated_by,
                json.dumps(fm, default=str),
            ),
        )

        # 3. Index Relations
        fm_relations = fm.get("relations")
        if isinstance(fm_relations, dict):
            for rel_type, targets in fm_relations.items():
                if isinstance(targets, list):
                    for t in targets:
                        con.execute(
                            "INSERT OR IGNORE INTO relations(source_rel, target_stem, relation_type) VALUES (?, ?, ?)",
                            (rel, str(t), str(rel_type)),
                        )
                elif isinstance(targets, str):
                    con.execute(
                        "INSERT OR IGNORE INTO relations(source_rel, target_stem, relation_type) VALUES (?, ?, ?)",
                        (rel, targets, str(rel_type)),
                    )

        # Wikilinks in body
        for m in re.findall(r"(?<!\\)\[\[([^\]\|]+)(?:\|[^\]]+)?\]\]", body):
            target_stem = Path(m.strip()).stem
            con.execute(
                "INSERT OR IGNORE INTO relations(source_rel, target_stem, relation_type) VALUES (?, ?, ?)",
                (rel, target_stem, "references"),
            )

        # 4. Index Invariants
        fm_invariants = fm.get("invariants")
        if isinstance(fm_invariants, list):
            for rule in fm_invariants:
                con.execute(
                    "INSERT INTO invariants(source_rel, rule) VALUES (?, ?)",
                    (rel, str(rule)),
                )
        inv_slice, _ = slice_markdown_section(text, "Invariants")
        if not inv_slice:
            inv_slice, _ = slice_markdown_section(text, "Non-Negotiable Invariants")
        if inv_slice:
            for ln in inv_slice.splitlines():
                if ln.strip().startswith("- "):
                    con.execute(
                        "INSERT INTO invariants(source_rel, rule) VALUES (?, ?)",
                        (rel, ln.strip()[2:].strip()),
                    )

        # 5. Index Verifications
        verif_blocks = [b.split("```")[0].strip() for b in body.split("```bash:verify")[1:]]
        for cmd in verif_blocks:
            if cmd.strip():
                con.execute(
                    "INSERT INTO verifications(source_rel, command) VALUES (?, ?)",
                    (rel, cmd.strip()),
                )

        # 6. Index Services from Services-Catalog.md
        default_host = get_machine_id()
        if rel == "40-Systems/Services-Catalog.md":
            for line in body.splitlines():
                if not line.strip().startswith("|") or ":---" in line or "Service / Stack" in line:
                    continue
                cols = [c.strip() for c in line.strip().split("|")[1:-1]]
                if len(cols) >= 5:
                    svc_name = re.sub(r"[*`]", "", cols[0]).strip()
                    container = cols[1].strip().strip("`")
                    ports = cols[2].strip()
                    replicas = cols[3].strip()
                    role = cols[4].strip()
                    con.execute(
                        """INSERT OR REPLACE INTO services(name, container_prefix, ports, replicas, role, host, network, rel_path)
                           VALUES (?, ?, ?, ?, ?, ?, ?, ?)""",
                        (svc_name, container, ports, replicas, role, default_host, "default", rel),
                    )
        elif fm.get("ports") or note_type == "service":
            ports_val = (
                json.dumps(fm.get("ports"), default=str)
                if isinstance(fm.get("ports"), list)
                else str(fm.get("ports") or "")
            )
            con.execute(
                """INSERT OR REPLACE INTO services(name, container_prefix, ports, replicas, role, host, network, rel_path)
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?)""",
                (
                    f.stem,
                    str(fm.get("container") or f"{f.stem}_*"),
                    ports_val,
                    "1",
                    summary,
                    host or default_host,
                    network or "default",
                    rel,
                ),
            )

    con.commit()


def execute_sql_query(vault: Path, sql: str) -> tuple[str, bool]:
    """Execute a read-only SQL query against the akatsuki index database."""
    clean_sql = sql.strip()
    norm = clean_sql.upper()
    if not (norm.startswith("SELECT") or norm.startswith("WITH") or norm.startswith("EXPLAIN")):
        return "Error: Only read-only queries (SELECT, WITH, EXPLAIN) are permitted.", True

    for forbidden in ("INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "CREATE", "ATTACH", "DETACH"):
        if re.search(rf"\b{forbidden}\b", norm):
            return f"Error: Mutating statement '{forbidden}' is forbidden.", True

    cache_dir = vault / ".akatsuki"
    cache_dir.mkdir(parents=True, exist_ok=True)
    db_path = cache_dir / "index.db"

    with fts_db_context(vault) as con_w:
        sync_fts_index(vault, con_w)

    con_ro = None
    try:
        con_ro = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True, timeout=10.0)
        con_ro.row_factory = sqlite3.Row
        cur = con_ro.execute(clean_sql)
        rows = [dict(r) for r in cur.fetchall()]
        return json.dumps(rows, indent=2, default=str), False
    except Exception as e:
        return f"SQL Error: {e!s}", True
    finally:
        if con_ro is not None:
            try:
                con_ro.close()
            except Exception:
                pass
