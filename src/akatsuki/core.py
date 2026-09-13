#!/usr/bin/env python3
"""
akatsuki — Universal CLI and Model Context Protocol (MCP) Server for the akatsuki Second Brain

Pure Agent Memory Substrate:
- High-entropy structured contracts & token budgeting.
- Relational SQLite WAL indexing with sub-millisecond exact property getters and SQL queries.
- Knowledge graph dependency traversal and architectural blast radius calculation.
- Machine-verifiable executable assertions (`bash:verify`) for living invariants.
- Surgical keypath setters and strict boundary schema linting under kernel lock.
- Native JSON-RPC 2.0 stdio MCP server for agy, claude, pi, and omp harnesses.
"""

import argparse
import ast
import datetime
import fcntl
import json
import os
import re
import socket
import sqlite3
import subprocess
import sys
from pathlib import Path

try:
    import yaml
    HAVE_PYYAML = True
except ImportError:
    HAVE_PYYAML = False


def get_machine_id() -> str:
    """Return resolved hostname or environment override identifier."""
    return (
        os.environ.get("AKATSUKI_HOST")
        or os.environ.get("HOSTNAME")
        or socket.gethostname().split(".")[0]
    )


def dump_frontmatter(fm: dict, body: str) -> str:
    """Serialize YAML frontmatter dictionary and prepend to body."""
    if HAVE_PYYAML:
        new_fm_str = yaml.dump(fm, sort_keys=False).strip()
    else:
        new_fm_lines = []
        for k, v in fm.items():
            if isinstance(v, list):
                new_fm_lines.append(f"{k}:")
                for item in v:
                    new_fm_lines.append(f"  - {item}")
            elif isinstance(v, dict):
                new_fm_lines.append(f"{k}: {json.dumps(v, default=str)}")
            else:
                new_fm_lines.append(f"{k}: {v}")
        new_fm_str = "\n".join(new_fm_lines)
    return f"---\n{new_fm_str}\n---\n" + body.lstrip()


RAW_EXTS = {
    ".yml", ".yaml", ".json", ".toml", ".sh", ".py", ".conf",
    ".sql", ".txt", ".service", ".timer", ".ini", ".cfg"
}


def is_raw_path(rel_path: str) -> bool:
    name = Path(rel_path).name.lower()
    if name in {"dockerfile", "caddyfile", "makefile"} or name.endswith(".example"):
        return True
    return any(name.endswith(ext) for ext in RAW_EXTS)

CURRENT_VAULT_OVERRIDE: Path | None = None


def resolve_vault_path(explicit_path: str | Path | None = None) -> Path:
    """Multi-tiered vault resolution:
    1. Explicitly passed argument (--vault / param)
    2. CURRENT_VAULT_OVERRIDE (set via CLI --vault)
    3. AKATSUKI_VAULT environment variable
    4. Upward directory walk from current working directory
    5. Well-known fallback paths (~/configs/knowledge-base/akatsuki, ~/.akatsuki, ~/akatsuki)
    6. Current working directory fallback
    """
    if explicit_path:
        return Path(explicit_path).expanduser().resolve()

    if CURRENT_VAULT_OVERRIDE:
        return CURRENT_VAULT_OVERRIDE

    env_vault = os.environ.get("AKATSUKI_VAULT")
    if not env_vault:
        cfg_env = Path.home() / ".config" / "knowledge-base" / "env"
        if cfg_env.is_file():
            try:
                for line in cfg_env.read_text(encoding="utf-8").splitlines():
                    clean = line.strip()
                    if clean.startswith("export AKATSUKI_VAULT=") or clean.startswith("AKATSUKI_VAULT="):
                        val = clean.split("=", 1)[1].strip().strip('"').strip("'")
                        val = os.path.expandvars(val)
                        if val and Path(val).is_dir():
                            env_vault = val
                            break
                    elif clean.startswith("export KNOWLEDGE_BASE_DIR=") or clean.startswith("KNOWLEDGE_BASE_DIR="):
                        val = clean.split("=", 1)[1].strip().strip('"').strip("'")
                        val = os.path.expandvars(val)
                        if val and (Path(val) / "akatsuki").is_dir():
                            env_vault = str(Path(val) / "akatsuki")
                            break
            except Exception:
                pass

    if env_vault:
        return Path(env_vault).expanduser().resolve()

    # Upward directory walk
    try:
        cwd = Path.cwd().resolve()
        for parent in [cwd, *cwd.parents]:
            if (parent / "akatsuki" / "40-Systems").is_dir() and (parent / "akatsuki" / "20-Projects").is_dir():
                return (parent / "akatsuki").resolve()
            if (parent / ".akatsuki").is_dir() or (
                (parent / "INDEX.md").is_file() and (parent / "AGENTS.md").is_file() and not (parent / "ejdertasimsi").is_dir()
            ):
                return parent
            if (parent / "40-Systems").is_dir() and (parent / "20-Projects").is_dir():
                return parent
    except Exception:
        pass

    # Well-known system locations
    home = Path.home()
    for candidate in [
        home / "Projects" / "fusuyfusuy" / "knowledge-base" / "akatsuki",
        home / "projects" / "fusuyfusuy" / "knowledge-base" / "akatsuki",
        home / "configs" / "knowledge-base" / "akatsuki",
        home / ".akatsuki",
        home / "akatsuki",
    ]:
        if candidate.exists() and candidate.is_dir():
            return candidate.resolve()

    return Path.cwd().resolve()


# ---------------------------------------------------------------------------
# Vault Core Helpers
# ---------------------------------------------------------------------------


def get_vault() -> Path:
    vault = resolve_vault_path()
    if not vault.exists():
        sys.stderr.write(
            f"Error: akatsuki vault path not found at {vault}\n"
            "Set $AKATSUKI_VAULT, specify --vault, or run 'akatsuki init' to bootstrap a new vault.\n"
        )
        sys.exit(1)
    return vault


def contained_path(vault: Path, rel_path: str) -> Path | None:
    """Resolve rel_path strictly inside the vault. Returns None on escape."""
    candidate = Path(rel_path.strip())
    if candidate.is_absolute():
        return None
    vault = vault.resolve()
    target = (vault / candidate).resolve()
    if target != vault and vault not in target.parents:
        return None
    return target


def parse_frontmatter(content: str) -> tuple[dict[str, object], str]:
    """Extract frontmatter and body from markdown content."""
    if not content.startswith("---"):
        return {}, content

    parts = content.split("---", 2)
    if len(parts) < 3:
        return {}, content

    fm_raw = parts[1]
    body = parts[2]

    if HAVE_PYYAML:
        try:
            data = yaml.safe_load(fm_raw)
            if isinstance(data, dict):
                return data, body
        except Exception:
            pass

    metadata: dict[str, object] = {}
    list_key: str | None = None
    for line in fm_raw.splitlines():
        stripped = line.strip()
        if stripped.startswith("- "):
            if list_key:
                metadata.setdefault(list_key, [])
                assert isinstance(metadata[list_key], list)
                metadata[list_key].append(stripped[2:].strip().strip('"').strip("'"))
            continue
        if ":" not in line or line != line.lstrip():
            list_key = None
            continue
        k, v = line.split(":", 1)
        key, val = k.strip(), v.strip()
        if key == "tags" and val.startswith("[") and val.endswith("]"):
            metadata[key] = [
                x.strip().strip('"').strip("'")
                for x in val[1:-1].split(",")
                if x.strip()
            ]
            list_key = None
        elif not val:
            metadata[key] = []
            list_key = key
        else:
            metadata[key] = val.strip('"').strip("'")
            list_key = None

    return metadata, body


def resolve_note_file(vault: Path, query: str) -> Path | None:
    """Resolve a note path by exact path, relative path, or stem strictly inside vault."""
    q = query.strip()
    if q.startswith("/") or q.startswith("~"):
        return None
    if q.endswith(".md"):
        q_stem = q[:-3]
    else:
        q_stem = q

    # 1. Exact path (strictly contained within vault)
    p = contained_path(vault, q)
    if p is not None and p.is_file():
        return p
    p_md = contained_path(vault, f"{q}.md")
    if p_md is not None and p_md.is_file():
        return p_md

    # 2. Match stem or path suffix within vault only
    candidates = []
    for f in vault.glob("**/*.md"):
        rel = str(f.relative_to(vault).with_suffix(""))
        if f.stem.lower() == q_stem.lower() or rel.lower() == q_stem.lower():
            candidates.append(f)

    if len(candidates) == 1:
        return candidates[0]
    elif len(candidates) > 1:
        for c in candidates:
            if c.stem == q_stem or str(c.relative_to(vault).with_suffix("")) == q_stem:
                return c
        return candidates[0]

    return None


# ---------------------------------------------------------------------------
# SQLite Index & Relational Store
# ---------------------------------------------------------------------------


def get_fts_db(vault: Path) -> sqlite3.Connection:
    """Connect to vault SQLite index, initializing tables if needed."""
    cache_dir = vault / ".akatsuki"
    cache_dir.mkdir(parents=True, exist_ok=True)
    db_path = cache_dir / "index.db"
    con = sqlite3.connect(str(db_path), timeout=30.0)
    con.row_factory = sqlite3.Row
    con.execute("PRAGMA journal_mode=WAL;")

    con.execute(
        "CREATE TABLE IF NOT EXISTS schema_meta (key TEXT PRIMARY KEY, val TEXT);"
    )
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

        con.execute(
            "CREATE TABLE file_meta (rel_path TEXT PRIMARY KEY, mtime REAL NOT NULL, size INTEGER NOT NULL);"
        )
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
        con.execute(
            "INSERT OR REPLACE INTO schema_meta(key, val) VALUES ('version', '5');"
        )
        con.commit()

    return con


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
            (rel, f.stem, domain, title, note_type, status, repo, host, network, summary, updated, updated_by, json.dumps(fm, default=str)),
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
                        (svc_name, container, ports, replicas, role, "TanriZarAtmaz", "dokploy-network", rel),
                    )
        elif fm.get("ports") or note_type == "service":
            ports_val = json.dumps(fm.get("ports"), default=str) if isinstance(fm.get("ports"), list) else str(fm.get("ports") or "")
            con.execute(
                """INSERT OR REPLACE INTO services(name, container_prefix, ports, replicas, role, host, network, rel_path)
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?)""",
                (f.stem, str(fm.get("container") or f"{f.stem}_*"), ports_val, "1", summary, host or "TanriZarAtmaz", network or "dokploy-network", rel),
            )

    con.commit()


def expand_query_term(w: str) -> list[str]:
    """Expand word with prefix and common morphological suffixes for high-recall matching."""
    clean_w = w.lower()
    terms = [f'"{clean_w}"*']
    if clean_w.endswith("ing") and len(clean_w) > 4:
        terms.append(f'"{clean_w[:-3]}"*')
    elif clean_w.endswith("ed") and len(clean_w) > 3:
        terms.append(f'"{clean_w[:-2]}"*')
        terms.append(f'"{clean_w[:-1]}"*')
    elif clean_w.endswith("s") and len(clean_w) > 3 and not clean_w.endswith("ss"):
        terms.append(f'"{clean_w[:-1]}"*')
    elif clean_w.endswith("ment") and len(clean_w) > 5:
        terms.append(f'"{clean_w[:-4]}"*')
    return list(dict.fromkeys(terms))


def build_fts_clause(words: list[str], op: str = "AND") -> str:
    """Combine expanded term groups with AND or OR operator."""
    groups = []
    for w in words:
        cands = expand_query_term(w)
        if len(cands) == 1:
            groups.append(cands[0])
        else:
            groups.append("(" + " OR ".join(cands) + ")")
    return f" {op} ".join(groups)


def search_vault(
    vault: Path, query: str, domain: str | None = None, limit: int | str = 10
) -> list[dict]:
    """Search notes in vault using Okapi BM25 ranking over SQLite FTS5 index."""
    clean_query = query.strip()
    if not clean_query:
        return []

    if limit is not None:
        try:
            limit = int(limit)
        except (ValueError, TypeError):
            limit = 10

    con = get_fts_db(vault)
    sync_fts_index(vault, con)

    words = re.findall(r"\w+", clean_query)
    if not words:
        return []

    if (
        clean_query.startswith('"')
        and clean_query.endswith('"')
        and len(clean_query) > 2
    ):
        phrase = clean_query.strip('"').replace('"', '""')
        queries_to_try = [f'"{phrase}"']
    else:
        and_query = build_fts_clause(words, op="AND")
        or_query = build_fts_clause(words, op="OR")
        queries_to_try = [and_query]
        if len(words) > 1:
            queries_to_try.append(or_query)

    domain_clause = "AND domain = ?" if domain else ""
    sql = f"""
        SELECT rel_path, stem, domain, title, summary,
               bm25(notes_fts, 0, 0, 0, 10.0, 5.0, 5.0, 1.0) as score,
               snippet(notes_fts, 6, '**', '**', '...', 12) as snippet
        FROM notes_fts
        WHERE notes_fts MATCH ? {domain_clause}
        ORDER BY score
        LIMIT ?
    """

    rows = []
    for q_candidate in queries_to_try:
        params = (q_candidate, domain, limit) if domain else (q_candidate, limit)
        try:
            cur = con.execute(sql, params)
            rows = cur.fetchall()
            if rows:
                break
        except Exception:
            continue

    results = []
    for r in rows:
        snip = r["snippet"].strip() if r["snippet"] else ""
        results.append(
            {
                "rel_path": r["rel_path"],
                "stem": r["stem"],
                "domain": r["domain"],
                "title": r["title"],
                "summary": r["summary"],
                "score": round(abs(r["score"]), 3),
                "snippet": snip,
                "matches": [(1, snip)] if snip else [],
            }
        )

    return results


# ---------------------------------------------------------------------------
# Slicing, Budgeting & Contract Extraction
# ---------------------------------------------------------------------------


def extract_headings(content: str) -> list[tuple[int, str, int]]:
    """Parse markdown content and return list of (level, heading_title, line_number)."""
    headings = []
    lines = content.splitlines()
    in_frontmatter = content.startswith("---")
    fm_dashes = 0
    for idx, line in enumerate(lines, 1):
        if in_frontmatter:
            if line.strip() == "---":
                fm_dashes += 1
                if fm_dashes == 2:
                    in_frontmatter = False
            continue

        m = re.match(r"^(#{1,6})\s+(.+)$", line)
        if m:
            level = len(m.group(1))
            heading_title = m.group(2).strip()
            headings.append((level, heading_title, idx))
    return headings


def normalize_heading(h: str) -> str:
    """Normalize heading for resilient matching by removing leading emojis and symbols."""
    return re.sub(r"^[^\w\s]+", "", h).strip().lower()


def slice_markdown_section(content: str, target: str) -> tuple[str | None, list[str]]:
    """Extract a markdown section matching target heading."""
    headings = extract_headings(content)
    toc_lines = [f"L{line_no}: {'#' * lvl} {title}" for lvl, title, line_no in headings]
    if not target or target.strip() == "__toc__":
        return None, toc_lines

    lines = content.splitlines()
    target_clean = normalize_heading(target)

    matched_idx = -1
    for i, (lvl, title, line_no) in enumerate(headings):
        if target_clean == normalize_heading(title) or target.lower() == title.lower():
            matched_idx = i
            break

    if matched_idx == -1:
        for i, (lvl, title, line_no) in enumerate(headings):
            if target_clean in normalize_heading(title) or target.lower() in title.lower():
                matched_idx = i
                break

    if matched_idx == -1:
        return None, toc_lines

    match_lvl, match_title, match_line_no = headings[matched_idx]
    start_line_idx = match_line_no - 1

    end_line_idx = len(lines)
    for next_lvl, next_title, next_line_no in headings[matched_idx + 1 :]:
        if next_lvl <= match_lvl:
            end_line_idx = next_line_no - 1
            break

    sliced_text = "\n".join(lines[start_line_idx:end_line_idx]).strip()
    return sliced_text, toc_lines


def apply_token_budget(text: str, budget: int | str | None) -> str:
    """Apply token budget packing to markdown text (heuristic ~4 chars/token)."""
    if budget is not None:
        try:
            budget = int(budget)
        except (ValueError, TypeError):
            budget = None
    if not budget or budget <= 0:
        return text
    char_budget = budget * 4
    if len(text) <= char_budget:
        return text

    lines = text.splitlines(keepends=True)
    packed = []
    current_chars = 0
    in_fm = text.startswith("---")
    fm_count = 0

    for line in lines:
        if in_fm:
            packed.append(line)
            current_chars += len(line)
            if line.strip() == "---":
                fm_count += 1
                if fm_count == 2:
                    in_fm = False
            continue

        if current_chars + len(line) > char_budget - 120:
            packed.append(f"\n[Notice: Output truncated to fit budget of ~{budget} tokens]\n")
            break
        packed.append(line)
        current_chars += len(line)

    return "".join(packed)


def extract_note_contract(vault: Path, note_query: str) -> tuple[str, bool]:
    """Extract machine-actionable boundary contract from a note."""
    target_file = resolve_note_file(vault, note_query)
    if not target_file:
        return f"Error: Note '{note_query}' not found.", True

    content = target_file.read_text(encoding="utf-8")
    fm, body = parse_frontmatter(content)
    stem = target_file.stem
    rel = str(target_file.relative_to(vault))

    invariants = fm.get("invariants") or []
    if not invariants:
        inv_slice, _ = slice_markdown_section(content, "Invariants")
        if not inv_slice:
            inv_slice, _ = slice_markdown_section(content, "Non-Negotiable Invariants")
        if inv_slice:
            invariants = [
                ln.strip()[2:].strip()
                for ln in inv_slice.splitlines()
                if ln.strip().startswith("- ")
            ]

    verif_blocks = [b.split("```")[0].strip() for b in body.split("```bash:verify")[1:]]
    clean_verifs = [v for v in verif_blocks if v]

    contract_data = {
        "stem": stem,
        "rel_path": rel,
        "title": fm.get("title", stem),
        "type": fm.get("type", "unknown"),
        "status": fm.get("status"),
        "repo": fm.get("repo"),
        "host": fm.get("host"),
        "network": fm.get("network"),
        "ports": fm.get("ports"),
        "relations": fm.get("relations", {}),
        "invariants": invariants,
        "verifications": clean_verifs,
        "summary": fm.get("summary", ""),
    }
    clean_data = {k: v for k, v in contract_data.items() if v is not None and v != "" and v != [] and v != {}}

    if HAVE_PYYAML:
        return yaml.dump(clean_data, sort_keys=False).strip(), False
    return json.dumps(clean_data, indent=2, default=str), False


# ---------------------------------------------------------------------------
# Relational Getters, Queries & Blast Radius
# ---------------------------------------------------------------------------


def get_keypath(vault: Path, keypath: str) -> tuple[str, bool]:
    """Retrieve exact property or entity at keypath."""
    con = get_fts_db(vault)
    sync_fts_index(vault, con)

    parts = [p.strip() for p in keypath.split(".") if p.strip()]
    if not parts:
        return "Error: Empty keypath.", True

    category = parts[0]
    if category == "services" and len(parts) >= 2:
        svc_name = parts[1]
        cur = con.execute("SELECT * FROM services WHERE name = ? OR name LIKE ?", (svc_name, f"{svc_name}%"))
        row = cur.fetchone()
        if not row:
            return f"Error: Service '{svc_name}' not found.", True
        data = dict(row)
        if len(parts) == 2:
            return json.dumps(data, indent=2, default=str), False
        prop = parts[2]
        if prop in data:
            val = data[prop]
            try:
                val = json.loads(val)
            except Exception:
                pass
            return (json.dumps(val, default=str) if not isinstance(val, str) else val), False
        return f"Error: Property '{prop}' not found in service '{svc_name}'.", True

    elif category == "entities" and len(parts) >= 2:
        ent_stem = parts[1]
        cur = con.execute("SELECT * FROM entities WHERE stem = ? OR rel_path = ?", (ent_stem, ent_stem))
        row = cur.fetchone()
        if not row:
            return f"Error: Entity '{ent_stem}' not found.", True
        meta = json.loads(row["metadata_json"])
        if len(parts) == 2:
            return json.dumps(meta, indent=2, default=str), False
        curr: object = meta
        for k in parts[2:]:
            if isinstance(curr, dict) and k in curr:
                curr = curr[k]
            else:
                return f"Error: Property '{k}' not found in '{keypath}'.", True
        return (json.dumps(curr, default=str) if not isinstance(curr, str) else curr), False

    # Otherwise resolve note by stem/path
    note_file = resolve_note_file(vault, category)
    if note_file:
        content = note_file.read_text(encoding="utf-8")
        fm, _ = parse_frontmatter(content)
        if len(parts) == 1:
            return json.dumps(fm, indent=2, default=str), False
        curr = fm
        for k in parts[1:]:
            if isinstance(curr, dict) and k in curr:
                curr = curr[k]
            else:
                return f"Error: Key '{k}' not found in '{note_file.name}'.", True
        return (json.dumps(curr, default=str) if not isinstance(curr, str) else curr), False

    return f"Error: Could not resolve keypath '{keypath}'.", True


def execute_sql_query(vault: Path, sql: str) -> tuple[str, bool]:
    """Execute a read-only SQL query against the akatsuki index database."""
    clean_sql = sql.strip()
    norm = clean_sql.upper()
    if not (norm.startswith("SELECT") or norm.startswith("WITH") or norm.startswith("EXPLAIN") or norm.startswith("PRAGMA")):
        return "Error: Only read-only queries (SELECT, WITH, EXPLAIN) are permitted.", True

    for forbidden in ("INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "CREATE", "ATTACH", "DETACH"):
        if re.search(rf"\b{forbidden}\b", norm):
            return f"Error: Mutating statement '{forbidden}' is forbidden.", True

    con = get_fts_db(vault)
    sync_fts_index(vault, con)

    try:
        cur = con.execute(clean_sql)
        rows = [dict(r) for r in cur.fetchall()]
        return json.dumps(rows, indent=2, default=str), False
    except Exception as e:
        return f"SQL Error: {str(e)}", True


def calculate_blast_radius(vault: Path, target: str) -> tuple[str, bool]:
    """Calculate upstream dependents, downstream dependencies, and boundary sinks for a target."""
    con = get_fts_db(vault)
    sync_fts_index(vault, con)

    t = target.strip()
    if t.endswith(".md"):
        t = t[:-3]
    t_stem = Path(t).stem

    cur = con.execute(
        "SELECT source_rel, relation_type FROM relations WHERE target_stem = ? OR target_stem LIKE ?",
        (t_stem, f"%{t_stem}%"),
    )
    upstream = cur.fetchall()

    cur = con.execute(
        "SELECT target_stem, relation_type FROM relations WHERE source_rel LIKE ? OR source_rel LIKE ?",
        (f"%{t_stem}.md", f"%{t_stem}/%"),
    )
    downstream = cur.fetchall()

    cur = con.execute(
        "SELECT name, ports, host, network, rel_path FROM services WHERE name = ? OR host = ? OR container_prefix LIKE ?",
        (t_stem, t_stem, f"%{t_stem}%"),
    )
    svcs = cur.fetchall()

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
            out.append(f"- **Service `{s['name']}`**: Ports: `{s['ports']}`, Host: `{s['host']}`, Network: `{s['network']}`")
    else:
        out.append("- *No discrete container/port allocation mapped.*")

    return "\n".join(out), False


def run_verification_tests(vault: Path, note_filter: str | None = None) -> tuple[str, bool]:
    """Execute machine-verifiable bash:verify assertion blocks in notes."""
    con = get_fts_db(vault)
    sync_fts_index(vault, con)

    if note_filter:
        stem = Path(note_filter).stem
        cur = con.execute(
            "SELECT source_rel, command FROM verifications WHERE source_rel LIKE ? OR source_rel LIKE ?",
            (f"%{stem}.md", f"%{stem}%"),
        )
    else:
        cur = con.execute("SELECT source_rel, command FROM verifications")

    rows = cur.fetchall()
    if not rows:
        return "No machine verification blocks (```bash:verify) found in target.", False

    results = []
    total = len(rows)
    passed = 0
    failed = 0

    for r in rows:
        src = r["source_rel"]
        cmd = r["command"].strip()
        try:
            res = subprocess.run(
                cmd,
                shell=True,
                capture_output=True,
                text=True,
                timeout=5,
            )
            ok = (res.returncode == 0)
            if ok:
                passed += 1
            else:
                failed += 1
            results.append({
                "source": src,
                "command": cmd,
                "exit_code": res.returncode,
                "passed": ok,
                "stdout": res.stdout.strip(),
                "stderr": res.stderr.strip(),
            })
        except subprocess.TimeoutExpired:
            failed += 1
            results.append({
                "source": src,
                "command": cmd,
                "exit_code": 124,
                "passed": False,
                "stdout": "",
                "stderr": "Command timed out after 5 seconds",
            })
        except Exception as e:
            failed += 1
            results.append({
                "source": src,
                "command": cmd,
                "exit_code": 1,
                "passed": False,
                "stdout": "",
                "stderr": str(e),
            })

    out = [f"Ran {total} verification assertion(s): {passed} PASSED, {failed} FAILED\n"]
    for res in results:
        status_icon = "✅" if res["passed"] else "❌"
        out.append(f"{status_icon} [{res['source']}] exit {res['exit_code']}: `{res['command']}`")
        if not res["passed"]:
            if res["stderr"]:
                out.append(f"    stderr: {res['stderr']}")
            if res["stdout"]:
                out.append(f"    stdout: {res['stdout']}")

    return "\n".join(out), (failed > 0)


def set_note_property(vault: Path, rel_path: str, keypath: str, value_str: str) -> tuple[str, bool]:
    """Surgically update a key-value property in frontmatter under kernel lock."""
    note_file = resolve_note_file(vault, rel_path)
    if not note_file:
        return f"Error: Note '{rel_path}' not found.", True

    try:
        parsed_val = json.loads(value_str)
    except Exception:
        parsed_val = value_str

    with VaultLock(vault):
        content = note_file.read_text(encoding="utf-8")
        fm, body = parse_frontmatter(content)

        keys = keypath.split(".")
        curr = fm
        for k in keys[:-1]:
            if k not in curr or not isinstance(curr[k], dict):
                curr[k] = {}
            curr = curr[k]
        curr[keys[-1]] = parsed_val

        if keypath not in ("updated", "updated_by"):
            fm["updated"] = datetime.datetime.now().astimezone().isoformat(timespec="seconds")
            fm["updated_by"] = get_machine_id()

        new_content = dump_frontmatter(fm, body)
        tmp_file = note_file.with_name(f".{note_file.name}.tmp.{os.getpid()}")
        tmp_file.write_text(new_content, encoding="utf-8")
        os.replace(tmp_file, note_file)

        try:
            db = get_fts_db(vault)
            sync_fts_index(vault, db)
        except Exception:
            pass

    if not str(rel_path).startswith("01-Daily"):
        try:
            append_work_log(vault, project="akatsuki", summary=f"set {note_file.name} {keypath}={value_str} -> exit 0")
        except Exception:
            pass

    return f"Successfully updated '{keypath}' in '{note_file.name}'.", False


def lint_vault(vault: Path) -> tuple[str, bool]:
    """Validate that all notes comply with strict machine schemas."""
    con = get_fts_db(vault)
    sync_fts_index(vault, con)

    cur = con.execute("SELECT rel_path, stem, type, metadata_json FROM entities")
    rows = cur.fetchall()
    errors = []

    required_by_type = {
        "project": ["title", "date", "type", "tags", "summary", "status"],
        "system": ["title", "date", "type", "tags", "summary"],
        "daily": ["title", "date", "type", "tags", "summary"],
        "agent": ["title", "date", "type", "tags", "summary"],
        "config": ["title", "date", "type", "tags", "summary"],
        "script": ["title", "date", "type", "tags", "summary"],
    }

    for r in rows:
        rel = r["rel_path"]
        note_type = r["type"]
        meta = json.loads(r["metadata_json"])
        needed = required_by_type.get(note_type, ["title", "date", "type", "summary"])
        missing = [f for f in needed if not meta.get(f)]
        if missing:
            errors.append(f"'{rel}' [{note_type}]: Missing required field(s): {', '.join(missing)}")

    # Validate syntax for non-markdown configs and scripts
    raw_files = []
    for d in ("50-Configs", "60-Scripts"):
        dp = vault / d
        if dp.exists():
            for f in dp.glob("**/*"):
                if f.is_file() and not f.name.startswith("."):
                    raw_files.append(f)

    for f in raw_files:
        rel = str(f.relative_to(vault))
        if f.suffix in (".yml", ".yaml"):
            try:
                if HAVE_PYYAML:
                    with open(f, "r", encoding="utf-8") as yf:
                        yaml.safe_load(yf)
            except Exception as e:
                errors.append(f"'{rel}': YAML syntax error: {e}")
        elif f.suffix == ".json":
            try:
                with open(f, "r", encoding="utf-8") as jf:
                    json.load(jf)
            except Exception as e:
                errors.append(f"'{rel}': JSON syntax error: {e}")
        elif f.suffix == ".py":
            try:
                with open(f, "r", encoding="utf-8") as pf:
                    ast.parse(pf.read(), filename=str(f))
            except Exception as e:
                errors.append(f"'{rel}': Python syntax error: {e}")
        elif f.suffix == ".sh":
            try:
                res = subprocess.run(["bash", "-n", str(f)], capture_output=True, text=True)
                if res.returncode != 0:
                    errors.append(f"'{rel}': Bash syntax error: {res.stderr.strip()}")
            except Exception:
                pass

    if errors:
        out = [f"FAILED: {len(errors)} lint violation(s) found in akatsuki:"]
        for e in errors[:15]:
            out.append(f"  - {e}")
        return "\n".join(out), True
    return f"PASSED: All {len(rows)} notes conform to schema specifications.", False


# ---------------------------------------------------------------------------
# Vault Mutation & Integrity
# ---------------------------------------------------------------------------


def verify_links(vault: Path) -> tuple[bool, list[tuple[str, str]]]:
    """Verify all wikilinks, markdown links, and note connectivity across the vault."""
    files = list(vault.glob("**/*.md"))
    valid_files = [
        f for f in files if not str(f.relative_to(vault)).startswith(("_templates", "."))
    ]

    stems = {f.stem.lower(): f for f in valid_files}
    rels = {str(f.relative_to(vault).with_suffix("")).lower(): f for f in valid_files}

    inbound_links = {str(f.relative_to(vault)): set() for f in valid_files}
    issues: list[tuple[str, str]] = []

    fence_pattern = r"```[\s\S]*?```"
    inline_pattern = r"`[^`\n]+`"

    for f in valid_files:
        src_rel = str(f.relative_to(vault))
        try:
            text = f.read_text(encoding="utf-8")
        except Exception as e:
            issues.append((src_rel, f"Unreadable file: {e}"))
            continue

        # Strip code blocks to prevent false positive matching on doc examples/audits
        clean_text = re.sub(fence_pattern, "", text)
        clean_text = re.sub(inline_pattern, "", clean_text)

        # 1. Wikilinks [[target|display]]
        for m in re.findall(r"(?<!\\)\[\[([^\]\|]+)(?:\|[^\]]+)?\]\]", clean_text):
            tgt = m.strip()
            tgt_clean = tgt[:-3] if tgt.endswith(".md") else tgt
            target_key = tgt_clean.lower()
            cand = rels.get(target_key) or stems.get(Path(tgt_clean).stem.lower())
            if cand is not None:
                inbound_links[str(cand.relative_to(vault))].add(src_rel)
            else:
                issues.append((src_rel, f"Broken wikilink: [[{tgt}]]"))

        # 2. Markdown links [text](path)
        for m in re.findall(r"\[(?:[^\]]*)\]\(([^)#\s]+)(?:#[^)]*)?\)", clean_text):
            tgt = m.strip()
            if tgt.startswith(("http://", "https://", "file://", "mailto:", "#")):
                continue
            resolved = (f.parent / tgt).resolve()
            if resolved.exists():
                if resolved.is_file() and str(resolved).startswith(str(vault)):
                    cand_rel = str(resolved.relative_to(vault))
                    if cand_rel in inbound_links:
                        inbound_links[cand_rel].add(src_rel)
            else:
                issues.append((src_rel, f"Broken markdown link: [{tgt}]"))

    # 3. Orphan Note Accounting
    root_anchors = {"INDEX.md", "OPERATOR.md", "AGENTS.md", "README.md"}
    for rel, inbounds in inbound_links.items():
        if rel in root_anchors or rel.startswith("01-Daily/"):
            continue
        if rel.startswith("50-Configs/") and not rel.endswith("Configs-MOC.md"):
            continue
        if rel.startswith("60-Scripts/") and not rel.endswith("Scripts-MOC.md"):
            continue
        if not inbounds:
            issues.append((rel, "Orphan note: No inbound links from MOC or index"))

    return len(issues) == 0, issues


def ensure_daily_note(vault: Path, date_str: str) -> Path:
    """Ensure daily note exists."""
    daily_dir = vault / "01-Daily"
    daily_dir.mkdir(parents=True, exist_ok=True)
    daily_note = daily_dir / f"{date_str}.md"

    if not daily_note.exists():
        template_file = vault / "_templates" / "daily-template.md"
        if template_file.exists():
            content = template_file.read_text(encoding="utf-8").replace(
                "{{date}}", date_str
            )
        else:
            content = f"""---
title: "{date_str}"
date: {date_str}
type: daily
tags:
  - daily-log
summary: "Daily log and activity ledger for {date_str}"
---

# 📅 {date_str}

## 🎯 Active Horizon & Daily Focus
- [ ] 

## 📝 Work Log & Session Notes
- 

## 💡 Insights, Architecture & Reflections
- 

## 🔗 Related Notes & Context
- [[INDEX]]
- [[OPERATOR]]
"""
        daily_note.write_text(content, encoding="utf-8")

    return daily_note


class VaultLock:
    """Process-safe kernel advisory lock for concurrent multi-agent mutations."""

    def __init__(self, vault: Path):
        self.lock_path = vault / ".akatsuki.lock"
        self._fd = None

    def __enter__(self):
        self._fd = open(self.lock_path, "a")
        fcntl.flock(self._fd.fileno(), fcntl.LOCK_EX)
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        if self._fd:
            try:
                fcntl.flock(self._fd.fileno(), fcntl.LOCK_UN)
                self._fd.close()
            except Exception:
                pass


def append_work_log(
    vault: Path, project: str, summary: str, device: str | None = None
) -> str:
    """Append a timestamped work log entry to today's daily note under kernel lock."""
    now = datetime.datetime.now().astimezone()
    date_str = now.strftime("%Y-%m-%d")
    time_str = now.strftime("%H:%M")
    dev = device or get_machine_id()

    with VaultLock(vault):
        daily_note = ensure_daily_note(vault, date_str)
        content = daily_note.read_text(encoding="utf-8")

        prefix = f"[{project}]" if project else ""
        dev_tag = f"[{dev}]" if dev else ""
        if dev_tag and prefix:
            header = f"- **{time_str}** {dev_tag}: {prefix} {summary}"
        elif dev_tag:
            header = f"- **{time_str}** {dev_tag}: {summary}"
        elif prefix:
            header = f"- **{time_str}**: {prefix} {summary}"
        else:
            header = f"- **{time_str}**: {summary}"
        entry = header.strip()

        target_heading = "## 📝 Work Log & Session Notes"
        if target_heading in content:
            head, tail = content.split(target_heading, 1)
            entry_line = f"{entry}\n"

            if not tail.strip():
                tail = "\n" + entry_line
            else:
                lines = tail.splitlines(keepends=True)
                at = next(
                    (
                        i
                        for i, ln in enumerate(lines)
                        if ln.startswith("#") and len(ln) - len(ln.lstrip("#")) <= 2
                    ),
                    len(lines),
                )
                body = lines[:at]
                while body and not body[-1].strip():
                    body.pop()
                if body and body[-1].strip() == "-":
                    body.pop()
                if body:
                    body.append("\n")
                body.append(entry_line)
                if at < len(lines):
                    body.append("\n")
                tail = "".join(body + lines[at:])

            new_content = head + target_heading + tail
        else:
            new_content = content.rstrip() + f"\n\n{target_heading}\n{entry}\n"

        tmp_file = daily_note.with_name(f".{daily_note.name}.tmp.{os.getpid()}")
        tmp_file.write_text(new_content, encoding="utf-8")
        os.replace(tmp_file, daily_note)

    return f"Recorded entry in {daily_note.name}: {entry}"


def validate_note_content(content: str) -> tuple[bool, str]:
    """Validate that note content has required frontmatter fields."""
    if not content.startswith("---"):
        return False, "Note must start with YAML frontmatter delimiter '---'."
    parts = content.split("---", 2)
    if len(parts) < 3:
        return False, "Note frontmatter is not closed with '---'."
    fm, _ = parse_frontmatter(content)
    required = ["title", "date", "type", "tags", "summary"]
    missing = [f for f in required if f not in fm]
    if missing:
        return False, f"Missing required frontmatter field(s): {', '.join(missing)}"
    return True, "Valid"


def auto_heal_frontmatter(content: str, rel_path: str) -> str:
    """Ensure markdown content has valid YAML frontmatter."""
    clean_rel = rel_path.strip().lstrip("/")
    stem = Path(clean_rel).stem
    date_str = datetime.datetime.now().astimezone().strftime("%Y-%m-%d")

    if clean_rel.startswith("20-Projects"):
        inferred_type = "project"
    elif clean_rel.startswith("40-Systems"):
        inferred_type = "system"
    elif clean_rel.startswith("30-Agents"):
        inferred_type = "agent"
    elif clean_rel.startswith("01-Daily"):
        inferred_type = "daily"
    elif clean_rel.startswith("90-Reference"):
        inferred_type = "reference"
    elif clean_rel.startswith("90-Database"):
        inferred_type = "database"
    else:
        inferred_type = "note"

    if content.startswith("---") and len(content.split("---", 2)) >= 3:
        fm, body = parse_frontmatter(content)
    else:
        fm, body = {}, content

    title = fm.get("title")
    if not title:
        for line in body.splitlines():
            if line.strip().startswith("# "):
                title = line.strip().lstrip("#").strip()
                break
        if not title:
            title = stem

    date_val = fm.get("date", date_str)
    type_val = fm.get("type", inferred_type)
    tags = fm.get("tags") or [inferred_type]
    if isinstance(tags, str):
        tags = [t.strip() for t in tags.split(",") if t.strip()]
    summary_val = fm.get("summary") or f"{title} overview and operational documentation."
    now_iso = datetime.datetime.now().astimezone().isoformat(timespec="seconds")
    dev = get_machine_id()
    updated_val = fm.get("updated", now_iso)
    updated_by_val = fm.get("updated_by", dev)

    fm_lines = [
        "---",
        f'title: "{title}"',
        f"date: {date_val}",
        f"type: {type_val}",
        "tags:",
    ]
    for t in tags:
        fm_lines.append(f"  - {t}")
    fm_lines.append(f'summary: "{summary_val}"')
    fm_lines.append(f'updated: "{updated_val}"')
    fm_lines.append(f'updated_by: "{updated_by_val}"')
    fm_lines.append("---\n")

    return "\n".join(fm_lines) + body.lstrip()


def append_section_to_note(
    vault: Path, rel_path: str, heading: str, content_to_append: str
) -> tuple[str, bool]:
    """Safely append content under a specific heading inside an existing note."""
    clean_rel = rel_path.strip()
    if clean_rel.startswith("/") or clean_rel.startswith("~"):
        return ("Error: Absolute paths are not accepted; use a vault-relative path.", True)
    if not clean_rel.endswith(".md"):
        clean_rel += ".md"

    target_file = resolve_note_file(vault, clean_rel)
    if not target_file:
        target_file = contained_path(vault, clean_rel)
        if target_file is None:
            return "Error: Path traversal outside vault boundary is forbidden.", True

    with VaultLock(vault):
        if not target_file.exists():
            stem = target_file.stem
            body = f"# {stem}\n\n## {heading}\n{content_to_append.strip()}\n"
            healed_content = auto_heal_frontmatter(body, clean_rel)
            target_file.parent.mkdir(parents=True, exist_ok=True)
            tmp_file = target_file.with_name(f".{target_file.name}.tmp.{os.getpid()}")
            tmp_file.write_text(healed_content, encoding="utf-8")
            os.replace(tmp_file, target_file)
            return f"Created '{clean_rel}' and added section '## {heading}'.", False

        orig_content = target_file.read_text(encoding="utf-8")
        headings = extract_headings(orig_content)
        lines = orig_content.splitlines()

        matched_idx = -1
        target_clean = normalize_heading(heading)
        for i, (lvl, title, line_no) in enumerate(headings):
            if target_clean == normalize_heading(title) or heading.lower() == title.lower():
                matched_idx = i
                break

        if matched_idx == -1:
            ranked = []
            for i, (lvl, title, line_no) in enumerate(headings):
                norm = normalize_heading(title)
                if target_clean in norm or heading.lower() in title.lower():
                    ranked.append((len(norm) - len(target_clean), i))
            if ranked:
                matched_idx = min(ranked)[1]

        if matched_idx != -1:
            match_lvl, match_title, match_line_no = headings[matched_idx]
            end_line_idx = len(lines)
            for next_lvl, next_title, next_line_no in headings[matched_idx + 1 :]:
                if next_lvl <= match_lvl:
                    end_line_idx = next_line_no - 1
                    break

            insert_lines = content_to_append.strip().splitlines()
            prefix = [""] if (end_line_idx > 0 and lines[end_line_idx - 1].strip() != "") else []
            suffix = [""]
            new_lines = (
                lines[:end_line_idx]
                + prefix
                + insert_lines
                + suffix
                + lines[end_line_idx:]
            )
            merged = "\n".join(new_lines).strip() + "\n"
            boundary = "\n".join(insert_lines)
            if boundary.strip():
                collapsed = re.sub(r"\n{3,}", "\n\n", boundary).strip()
                merged = merged.replace(boundary, collapsed, 1)
            new_content = merged
        else:
            new_content = orig_content.rstrip() + f"\n\n## {heading}\n{content_to_append.strip()}\n"

        fm, body = parse_frontmatter(new_content)
        if fm:
            fm["updated"] = datetime.datetime.now().astimezone().isoformat(timespec="seconds")
            fm["updated_by"] = get_machine_id()
            new_content = dump_frontmatter(fm, body)

        tmp_file = target_file.with_name(f".{target_file.name}.tmp.{os.getpid()}")
        tmp_file.write_text(new_content, encoding="utf-8")
        os.replace(tmp_file, target_file)

        try:
            db = get_fts_db(vault)
            sync_fts_index(vault, db)
        except Exception:
            pass

    if not clean_rel.startswith("01-Daily"):
        try:
            append_work_log(vault, project="akatsuki", summary=f"append section '{heading}' in {clean_rel} -> exit 0")
        except Exception:
            pass

    v_ok, broken = verify_links(vault)
    report = f"Successfully appended to '## {heading}' in '{target_file.name}'."
    if not v_ok:
        report += f" Warning: {len(broken)} broken link(s) detected in vault."
    return report, False


def write_note(
    vault: Path, rel_path: str, content: str, overwrite: bool = False, raw: bool = False
) -> tuple[str, bool]:
    """Write note or raw config/script into vault safely enforcing boundary, permissions, and kernel lock."""
    clean_rel = rel_path.strip()
    if clean_rel.startswith("/") or clean_rel.startswith("~"):
        return ("Error: Absolute paths are not accepted; use a vault-relative path.", True)

    is_raw = raw or is_raw_path(clean_rel)
    if not is_raw and not clean_rel.endswith(".md"):
        clean_rel += ".md"

    target = contained_path(vault, clean_rel)
    if target is None:
        return "Error: Path traversal outside vault boundary is forbidden.", True

    if is_raw:
        final_content = content
    else:
        ok, _ = validate_note_content(content)
        if ok:
            fm, body = parse_frontmatter(content)
            fm["updated"] = datetime.datetime.now().astimezone().isoformat(timespec="seconds")
            fm["updated_by"] = get_machine_id()
            final_content = dump_frontmatter(fm, body)
        else:
            final_content = auto_heal_frontmatter(content, clean_rel)

    with VaultLock(vault):
        was_existing = target.exists()
        if was_existing and not overwrite:
            return (f"Error: File already exists at '{clean_rel}'. Set overwrite=True to replace.", True)

        target.parent.mkdir(parents=True, exist_ok=True)
        tmp_file = target.with_name(f".{target.name}.tmp.{os.getpid()}")
        tmp_file.write_text(final_content, encoding="utf-8")
        if is_raw and (clean_rel.startswith("60-Scripts/") or clean_rel.endswith((".sh", ".py"))):
            try:
                tmp_file.chmod(0o755)
            except Exception:
                pass
        os.replace(tmp_file, target)

        if not is_raw:
            try:
                db = get_fts_db(vault)
                sync_fts_index(vault, db)
            except Exception:
                pass

    if not clean_rel.startswith("01-Daily"):
        try:
            action = "overwrite" if was_existing else "create"
            append_work_log(vault, project="akatsuki", summary=f"{action} {clean_rel} -> exit 0")
        except Exception:
            pass

    if is_raw:
        return f"Successfully wrote raw file '{clean_rel}'.", False

    v_ok, broken = verify_links(vault)
    report = f"Successfully wrote '{clean_rel}'."
    if not v_ok:
        report += f" Warning: {len(broken)} broken link(s) detected in vault."
    return report, False


def list_notes_in_vault(vault: Path, domain: str | None = None) -> list[dict]:
    """List notes in vault with metadata."""
    notes = []
    for f in sorted(vault.glob("**/*.md")):
        rel = str(f.relative_to(vault))
        if rel.startswith("_templates") or rel.startswith("."):
            continue
        if domain and not rel.startswith(domain):
            continue
        try:
            text = f.read_text(encoding="utf-8")
            fm, _ = parse_frontmatter(text)
            notes.append(
                {
                    "rel_path": rel,
                    "stem": f.stem,
                    "title": fm.get("title", f.stem),
                    "type": fm.get("type", "note"),
                    "summary": fm.get("summary", ""),
                }
            )
        except Exception:
            continue
    return notes


# ---------------------------------------------------------------------------
# CLI Command Implementations
# ---------------------------------------------------------------------------


def cli_search(args):
    vault = get_vault()
    results = search_vault(vault, args.query, domain=args.domain, limit=args.limit)
    if not results:
        print(f"No notes found matching '{args.query}'.")
        return

    print(f"Found {len(results)} note(s) matching '{args.query}' [Okapi BM25]:\n")
    for r in results:
        print(f"📄 {r['title']} ({r['rel_path']}) [BM25: {r['score']}]")
        if r["summary"]:
            print(f"   Summary: {r['summary']}")
        if r["snippet"]:
            print(f"   Excerpt: {r['snippet']}")
        print()


def cli_read(args):
    vault = get_vault()
    note_file = resolve_note_file(vault, args.note)
    if not note_file:
        print(f"Error: Note '{args.note}' not found in akatsuki vault.", file=sys.stderr)
        sys.exit(1)

    content = note_file.read_text(encoding="utf-8")
    if getattr(args, "toc", False):
        _, toc_lines = slice_markdown_section(content, "__toc__")
        print(f"Table of Contents for {note_file.stem}:\n")
        for line in toc_lines:
            print(f"  {line}")
        return

    if getattr(args, "section", None):
        sliced, toc_lines = slice_markdown_section(content, args.section)
        if sliced is None:
            print(f"Section '{args.section}' not found in '{note_file.name}'.", file=sys.stderr)
            print("Available sections:\n" + "\n".join(f"  - {line}" for line in toc_lines), file=sys.stderr)
            sys.exit(1)
        content = sliced

    budget = getattr(args, "budget", None)
    if budget:
        content = apply_token_budget(content, budget)

    print(content)


def cli_contract(args):
    vault = get_vault()
    contract, is_err = extract_note_contract(vault, args.note)
    if is_err:
        print(contract, file=sys.stderr)
        sys.exit(1)
    print(contract)


def cli_get(args):
    vault = get_vault()
    val, is_err = get_keypath(vault, args.keypath)
    if is_err:
        print(val, file=sys.stderr)
        sys.exit(1)
    print(val)


def cli_query(args):
    vault = get_vault()
    out, is_err = execute_sql_query(vault, args.sql)
    if is_err:
        print(out, file=sys.stderr)
        sys.exit(1)
    print(out)


def cli_blast(args):
    vault = get_vault()
    out, is_err = calculate_blast_radius(vault, args.target)
    if is_err:
        print(out, file=sys.stderr)
        sys.exit(1)
    print(out)


def cli_test(args):
    vault = get_vault()
    out, is_err = run_verification_tests(vault, note_filter=args.note)
    print(out)
    if is_err:
        sys.exit(1)


def cli_set(args):
    vault = get_vault()
    msg, is_err = set_note_property(vault, args.note, args.key, args.value)
    if is_err:
        print(msg, file=sys.stderr)
        sys.exit(1)
    print(msg)


def cli_lint(args):
    vault = get_vault()
    out, is_err = lint_vault(vault)
    if is_err:
        print(out, file=sys.stderr)
        sys.exit(1)
    print(out)


def cli_append(args):
    vault = get_vault()
    content = args.content
    if not content:
        content = Path(args.file).read_text(encoding="utf-8") if args.file else sys.stdin.read()

    if not content or not content.strip():
        print("Error: No content provided to append.", file=sys.stderr)
        sys.exit(1)

    msg, is_err = append_section_to_note(vault, args.note, args.heading, content)
    if is_err:
        print(msg, file=sys.stderr)
        sys.exit(1)
    print(msg)


def cli_services(args):
    vault = get_vault()
    con = get_fts_db(vault)
    sync_fts_index(vault, con)
    cur = con.execute("SELECT name, container_prefix, ports, replicas, role, host, network FROM services")
    rows = [dict(r) for r in cur.fetchall()]
    if rows:
        print(json.dumps(rows, indent=2, default=str))
        return
    catalog = vault / "40-Systems" / "Services-Catalog.md"
    if catalog.exists():
        print(catalog.read_text(encoding="utf-8"))


def cli_projects(args):
    vault = get_vault()
    con = get_fts_db(vault)
    sync_fts_index(vault, con)
    cur = con.execute("SELECT stem, title, status, repo, host, network, summary FROM entities WHERE type = 'project'")
    rows = [dict(r) for r in cur.fetchall()]
    if rows:
        print(json.dumps(rows, indent=2, default=str))
        return
    moc = vault / "20-Projects" / "Projects-MOC.md"
    if not moc.exists():
        moc = vault / "INDEX.md"
    print(moc.read_text(encoding="utf-8"))


def cli_daily(args):
    vault = get_vault()
    if args.date:
        target_date = args.date.strip()
        if not re.match(r"^\d{4}-\d{2}-\d{2}$", target_date):
            print(f"Error: Invalid date format '{target_date}'. Expected YYYY-MM-DD.", file=sys.stderr)
            sys.exit(1)
    else:
        target_date = datetime.datetime.now().astimezone().strftime("%Y-%m-%d")
    note = vault / "01-Daily" / f"{target_date}.md"
    if not note.exists():
        print(f"Daily note for {target_date} does not exist yet.")
        return
    print(note.read_text(encoding="utf-8"))


def cli_log(args):
    vault = get_vault()
    device = getattr(args, "device", None)
    res = append_work_log(vault, args.project, args.summary, device=device)
    print(res)
    ok, broken = verify_links(vault)
    if not ok:
        print(f"Warning: {len(broken)} link/graph issue(s) detected in vault.", file=sys.stderr)


def cli_verify(args):
    vault = get_vault()
    ok, broken = verify_links(vault)
    if not ok:
        print(f"FAILED: {len(broken)} link/graph issue(s) found in akatsuki:", file=sys.stderr)
        for src, issue in broken[:15]:
            print(f"  - In '{src}': {issue}", file=sys.stderr)
        sys.exit(1)
    print("PASSED: All wikilinks and markdown links in akatsuki resolve cleanly (zero orphans).")


def cli_list(args):
    vault = get_vault()
    notes = list_notes_in_vault(vault, domain=args.domain)
    if not notes:
        print("No notes found.")
        return
    print(f"Notes in akatsuki ({len(notes)}):\n")
    for n in notes:
        print(f"• {n['stem']} [{n['type']}] ({n['rel_path']})")
        if n["summary"]:
            print(f"    {n['summary']}")


def cli_write(args):
    vault = get_vault()
    content = Path(args.file).read_text(encoding="utf-8") if args.file else sys.stdin.read()
    raw_flag = getattr(args, "raw", False)
    msg, is_err = write_note(vault, args.path, content, overwrite=args.overwrite, raw=raw_flag)
    if is_err:
        print(msg, file=sys.stderr)
        sys.exit(1)
    print(msg)


# ---------------------------------------------------------------------------
# Model Context Protocol (MCP) Server Implementation
# ---------------------------------------------------------------------------

MCP_TOOLS = [
    {
        "name": "akatsuki_search",
        "description": "Search notes, architecture specifications, and infrastructure configs in akatsuki using Okapi BM25 ranking, morphological suffix expansion, and excerpt snippets.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search terms, topic, or exact phrase in quotes.",
                },
                "domain": {
                    "type": "string",
                    "enum": [
                        "00-Meta",
                        "01-Daily",
                        "20-Projects",
                        "30-Agents",
                        "40-Systems",
                        "50-Configs",
                        "60-Scripts",
                        "90-Reference",
                        "90-Database",
                    ],
                    "description": "Optional domain directory to restrict search scope.",
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of search results to return (default: 10).",
                },
            },
            "required": ["query"],
        },
    },
    {
        "name": "akatsuki_read",
        "description": "Read contents or section of an akatsuki note by name/path. Supports token budgeting to prevent context bloating.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "note": {
                    "type": "string",
                    "description": "Note title, stem, or relative path (e.g. 'Dokploy-Traefik', '20-Projects/bountools').",
                },
                "section": {
                    "type": "string",
                    "description": "Optional section heading name to extract.",
                },
                "budget": {
                    "type": "integer",
                    "description": "Optional token budget (e.g. 200) to cap context size heuristically.",
                },
            },
            "required": ["note"],
        },
    },
    {
        "name": "akatsuki_contract",
        "description": "Extract the strict machine-actionable boundary contract (frontmatter, ports, relations, invariants, verifications) of a note without human narrative prose.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "note": {
                    "type": "string",
                    "description": "Note title, stem, or relative path (e.g. 'bountools', 'Dokploy-Traefik').",
                }
            },
            "required": ["note"],
        },
    },
    {
        "name": "akatsuki_get",
        "description": "O(1) exact property getter. Query sub-properties like 'services.bountools.ports', 'entities.filament.repo', or '40-Systems/TanriZarAtmaz-Host.host'.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "Dot-separated keypath (e.g. 'services.bountools.ports', 'entities.filament.repo').",
                }
            },
            "required": ["key"],
        },
    },
    {
        "name": "akatsuki_query",
        "description": "Execute a read-only SQL query against the SQLite index database tables: entities, services, relations, invariants, verifications.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "sql": {
                    "type": "string",
                    "description": "Read-only SQL query (e.g. 'SELECT name, ports, host FROM services WHERE host = \"TanriZarAtmaz\"').",
                }
            },
            "required": ["sql"],
        },
    },
    {
        "name": "akatsuki_blast",
        "description": "Calculate architectural blast radius for a service, component, or system. Returns upstream dependents, downstream dependencies, and boundary sinks.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "description": "Component or note stem (e.g. 'Dokploy-Traefik', 'TanriZarAtmaz', 'bountools').",
                }
            },
            "required": ["target"],
        },
    },
    {
        "name": "akatsuki_test",
        "description": "Execute machine-verifiable (```bash:verify) assertion blocks inside notes to verify that operational invariants hold true on the host.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "note": {
                    "type": "string",
                    "description": "Optional note stem or relative path to test. If omitted, runs all verifications in vault.",
                }
            },
        },
    },
    {
        "name": "akatsuki_set",
        "description": "Surgically update a key-value property in a note's frontmatter under kernel lock without rewriting the full file.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "note": {
                    "type": "string",
                    "description": "Note title, stem, or relative path (e.g. '20-Projects/bountools').",
                },
                "key": {
                    "type": "string",
                    "description": "Dot-separated keypath to update (e.g. 'status', 'relations.depends_on').",
                },
                "value": {
                    "type": "string",
                    "description": "New value as string, JSON list, or JSON dict.",
                },
            },
            "required": ["note", "key", "value"],
        },
    },
    {
        "name": "akatsuki_lint",
        "description": "Validate that all notes comply with strict machine schemas (valid types, required frontmatter fields, zero orphan specs).",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "akatsuki_append_section",
        "description": "Append markdown content directly under a specific heading in an akatsuki note without rewriting the entire file.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "note": {
                    "type": "string",
                    "description": "Note title, stem, or relative path.",
                },
                "heading": {
                    "type": "string",
                    "description": "Heading name under which to append content.",
                },
                "content": {
                    "type": "string",
                    "description": "Markdown content to append under the heading.",
                },
            },
            "required": ["note", "heading", "content"],
        },
    },
    {
        "name": "akatsuki_services",
        "description": "Retrieve the active self-hosted services catalog, container tables, and port allocations as structured JSON.",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "akatsuki_projects",
        "description": "Retrieve inventory of active software projects, tools, and technical architectures as structured JSON.",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "akatsuki_daily",
        "description": "Retrieve the daily operational note and focus horizons for today (or a specified YYYY-MM-DD date).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "date": {
                    "type": "string",
                    "description": "Optional date in YYYY-MM-DD format. Defaults to today.",
                }
            },
        },
    },
    {
        "name": "akatsuki_record_log",
        "description": "Deposit and record a completed unit of work, commit hash, or deployment into today's akatsuki daily log.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "project": {
                    "type": "string",
                    "description": "The project or repository name.",
                },
                "summary": {
                    "type": "string",
                    "description": "Punchy summary of changes made, commit hashes, or test results.",
                },
                "device": {
                    "type": "string",
                    "description": "Optional device or hostname identifier. Defaults to host machine.",
                },
            },
            "required": ["summary"],
        },
    },
    {
        "name": "akatsuki_write_note",
        "description": "Write or update an architectural note, config file, or script in akatsuki. Enforces path boundaries and supports raw formats.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Relative path within akatsuki (e.g. '20-Projects/my-app.md', '50-Configs/traefik.yml', '60-Scripts/deploy.sh').",
                },
                "content": {
                    "type": "string",
                    "description": "Content string to write.",
                },
                "overwrite": {
                    "type": "boolean",
                    "description": "Set to true to overwrite an existing file.",
                },
                "raw": {
                    "type": "boolean",
                    "description": "Set to true to write raw non-markdown configuration or script files (.yml, .sh, .py, etc.) without forcing .md extension or auto-healing frontmatter.",
                },
            },
            "required": ["path", "content"],
        },
    },
    {
        "name": "akatsuki_verify",
        "description": "Verify wikilink integrity and report broken references across the entire akatsuki knowledge graph.",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "akatsuki_list_notes",
        "description": "List existing notes in akatsuki with metadata, optionally filtered by domain.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "domain": {
                    "type": "string",
                    "enum": [
                        "00-Meta",
                        "01-Daily",
                        "20-Projects",
                        "30-Agents",
                        "40-Systems",
                        "50-Configs",
                        "60-Scripts",
                        "90-Reference",
                        "90-Database",
                    ],
                    "description": "Optional domain directory filter.",
                }
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
        results = search_vault(vault, query, domain=domain, limit=limit)
        if not results:
            return f"No notes found in akatsuki matching '{query}'.", False
        out = [f"Found {len(results)} matching note(s) using Okapi BM25:"]
        for r in results:
            out.append(
                f"\n- **{r['title']}** (`{r['rel_path']}`) [BM25 score: {r['score']}]: {r['summary']}"
            )
            if r["snippet"]:
                out.append(f"    Excerpt: {r['snippet']}")
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
        return extract_note_contract(vault, note)

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
        return calculate_blast_radius(vault, target)

    elif name == "akatsuki_test":
        note = args.get("note")
        return run_verification_tests(vault, note_filter=note)

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

    elif name == "akatsuki_services":
        con = get_fts_db(vault)
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
        con = get_fts_db(vault)
        sync_fts_index(vault, con)
        cur = con.execute("SELECT stem, title, status, repo, host, network, summary FROM entities WHERE type = 'project'")
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

    return f"Unknown tool: {name}", True


MCP_RESOURCES = [
    {
        "uri": "akatsuki://services",
        "name": "Live Services Catalog",
        "description": "Active containerized services, port allocations, and ingress routing from TanriZarAtmaz.",
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
]


def handle_mcp_resource_read(uri: str) -> tuple[str, bool]:
    """Resolve and read an akatsuki:// resource URI."""
    vault = get_vault()
    if uri == "akatsuki://services":
        con = get_fts_db(vault)
        sync_fts_index(vault, con)
        cur = con.execute("SELECT name, container_prefix, ports, replicas, role, host, network FROM services")
        rows = [dict(r) for r in cur.fetchall()]
        return json.dumps(rows, indent=2, default=str), False
    elif uri == "akatsuki://projects":
        con = get_fts_db(vault)
        sync_fts_index(vault, con)
        cur = con.execute("SELECT stem, title, status, repo, host, network, summary FROM entities WHERE type = 'project'")
        rows = [dict(r) for r in cur.fetchall()]
        return json.dumps(rows, indent=2, default=str), False
    elif uri == "akatsuki://operator":
        f = vault / "OPERATOR.md"
        if not f.exists():
            f = vault / "00-Meta" / "OPERATOR.md"
        return (
            (f.read_text(encoding="utf-8"), False)
            if f.exists()
            else ("OPERATOR note not found.", True)
        )
    elif uri == "akatsuki://daily":
        today = datetime.datetime.now().astimezone().strftime("%Y-%m-%d")
        daily_note = ensure_daily_note(vault, today)
        return daily_note.read_text(encoding="utf-8"), False
    elif uri.startswith("akatsuki://"):
        stem_or_path = uri.replace("akatsuki://", "")
        note_file = resolve_note_file(vault, stem_or_path)
        if note_file and note_file.exists():
            return note_file.read_text(encoding="utf-8"), False
        return f"Resource '{uri}' not found in akatsuki vault.", True
    return f"Unsupported resource URI scheme: '{uri}'", True


def dispatch_single_request(req: dict) -> dict | None:
    """Dispatch a single JSON-RPC 2.0 request or notification dictionary."""
    if not isinstance(req, dict):
        return {
            "jsonrpc": "2.0",
            "id": None,
            "error": {"code": -32600, "message": "Invalid Request: Expected JSON object"},
        }

    is_notification = "id" not in req
    req_id = req.get("id")
    method = req.get("method")
    raw_params = req.get("params")
    params = raw_params if isinstance(raw_params, dict) else {}

    if method == "initialize":
        if is_notification:
            return None
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {},
                    "resources": {"subscribe": False, "listChanged": False},
                },
                "serverInfo": {"name": "akatsuki", "version": "3.0.0"},
            },
        }
    elif method == "notifications/initialized":
        return None
    elif method == "ping":
        if is_notification:
            return None
        return {"jsonrpc": "2.0", "id": req_id, "result": {}}
    elif method == "tools/list":
        if is_notification:
            return None
        return {"jsonrpc": "2.0", "id": req_id, "result": {"tools": MCP_TOOLS}}
    elif method == "resources/list":
        if is_notification:
            return None
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {"resources": MCP_RESOURCES},
        }
    elif method == "resources/templates/list":
        if is_notification:
            return None
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "resourceTemplates": [
                    {
                        "uriTemplate": "akatsuki://{note}",
                        "name": "Akatsuki Note",
                        "description": "Read any note, system spec, or project contract in the vault by stem or relative path.",
                        "mimeType": "text/markdown",
                    }
                ]
            },
        }
    elif method == "resources/read":
        if is_notification:
            return None
        uri = params.get("uri", "")
        try:
            text_out, is_err = handle_mcp_resource_read(uri)
        except Exception as e:
            text_out, is_err = f"Error reading resource '{uri}': {str(e)}", True
        mime = (
            "application/json"
            if uri in ("akatsuki://services", "akatsuki://projects")
            else "text/markdown"
        )
        if is_err:
            return {
                "jsonrpc": "2.0",
                "id": req_id,
                "error": {"code": -32602, "message": text_out},
            }
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "contents": [
                    {"uri": uri, "mimeType": mime, "text": text_out}
                ]
            },
        }
    elif method == "tools/call":
        tool_name = params.get("name")
        tool_args = params.get("arguments", {})
        if not isinstance(tool_args, dict):
            tool_args = {}
        try:
            text_out, is_err = handle_mcp_call(tool_name, tool_args)
            if is_notification:
                return None
            return {
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "content": [{"type": "text", "text": text_out}],
                    "isError": is_err,
                },
            }
        except Exception as e:
            if is_notification:
                return None
            return {
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "content": [
                        {
                            "type": "text",
                            "text": f"Exception in {tool_name}: {str(e)}",
                        }
                    ],
                    "isError": True,
                },
            }
    elif not is_notification:
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "error": {"code": -32601, "message": f"Method not found: {method}"},
        }
    return None


def run_mcp_server():
    """Run JSON-RPC 2.0 stdio loop supporting batch and single frames."""
    sys.stderr.write("akatsuki MCP server running on stdio\n")
    sys.stderr.flush()

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except Exception:
            resp = {
                "jsonrpc": "2.0",
                "id": None,
                "error": {"code": -32700, "message": "Parse error: Invalid JSON"},
            }
            sys.stdout.write(json.dumps(resp) + "\n")
            sys.stdout.flush()
            continue

        if isinstance(req, list):
            if not req:
                resp = {
                    "jsonrpc": "2.0",
                    "id": None,
                    "error": {"code": -32600, "message": "Invalid Request: Empty batch array"},
                }
                sys.stdout.write(json.dumps(resp) + "\n")
                sys.stdout.flush()
                continue
            batch_resps = []
            for item in req:
                try:
                    single_resp = dispatch_single_request(item)
                    if single_resp is not None:
                        batch_resps.append(single_resp)
                except Exception as e:
                    item_id = item.get("id") if isinstance(item, dict) else None
                    batch_resps.append({
                        "jsonrpc": "2.0",
                        "id": item_id,
                        "error": {"code": -32603, "message": f"Internal server error: {e}"},
                    })
            if batch_resps:
                sys.stdout.write(json.dumps(batch_resps) + "\n")
                sys.stdout.flush()
        elif isinstance(req, dict):
            try:
                resp = dispatch_single_request(req)
                if resp is not None:
                    sys.stdout.write(json.dumps(resp) + "\n")
                    sys.stdout.flush()
            except Exception as e:
                resp = {
                    "jsonrpc": "2.0",
                    "id": req.get("id"),
                    "error": {"code": -32603, "message": f"Internal server error: {e}"},
                }
                sys.stdout.write(json.dumps(resp) + "\n")
                sys.stdout.flush()
        else:
            resp = {
                "jsonrpc": "2.0",
                "id": None,
                "error": {"code": -32600, "message": "Invalid Request: Expected JSON object or array"},
            }
            sys.stdout.write(json.dumps(resp) + "\n")
            sys.stdout.flush()


def cli_init(args: argparse.Namespace) -> None:
    target = Path(args.path).expanduser().resolve()
    target.mkdir(parents=True, exist_ok=True)

    akatsuki_dir = target / ".akatsuki"
    akatsuki_dir.mkdir(exist_ok=True)

    subdirs = [
        "00-Meta",
        "01-Daily",
        "20-Projects",
        "30-Agents",
        "40-Systems",
        "50-Configs",
        "90-Reference",
        "90-Database",
        "_templates",
    ]
    for s in subdirs:
        (target / s).mkdir(exist_ok=True)

    gitignore = target / ".gitignore"
    if not gitignore.exists():
        gitignore.write_text(
            "# Obsidian workspace & caches\n"
            ".obsidian/workspace*\n"
            ".obsidian/cache/\n"
            ".obsidian/graph.json\n"
            ".obsidian/plugins/*/data.json.bak\n\n"
            "# Temporary & local agent scratch files\n"
            "*.tmp\n"
            ".akatsuki.lock\n"
            ".akatsuki/\n"
            ".drafts/\n"
            ".scratch/\n\n"
            "# Machine-specific AST indexing & runtime sessions\n"
            ".mimori/\n"
            ".pi/\n"
            ".pi-glla/\n"
            ".claude/\n"
            ".codex/\n"
            ".gemini/\n"
            ".omp/\n\n"
            "# Secrets\n"
            ".env\n"
            "*.key\n"
            "*.pem\n"
            "*.secret\n",
            encoding="utf-8",
        )

    agents_md = target / "AGENTS.md"
    if not agents_md.exists():
        agents_md.write_text(
            "# AGENTS.md — System Protocol & Living Architecture Invariants\n\n"
            "## Core Invariants\n"
            "- **Architecture at the Boundary**: Keep system contracts explicit and strict.\n"
            "- **Living Invariant Verification**: All assertions under `bash:verify` must evaluate to exit code 0.\n"
            "- **Telegraphic Caveman Logging**: Append timestamped ledger entries to daily notes upon completing tasks.\n",
            encoding="utf-8",
        )

    index_md = target / "INDEX.md"
    if not index_md.exists():
        today_str = datetime.date.today().isoformat()
        index_md.write_text(
            f"---\ntitle: Living System Catalog\ndate: {today_str}\ntype: index\nsummary: Master index of living systems architecture.\n---\n\n"
            "# 🏛️ Living System Catalog\n\n"
            "## 📌 Overview\n"
            "Central catalog and index for systems architecture, service contracts, and operational ledgers.\n",
            encoding="utf-8",
        )

    print(f"✅ Initialized fresh Akatsuki living memory vault at: {target}")


# ---------------------------------------------------------------------------
# Main Router
# ---------------------------------------------------------------------------


def main():
    parser = argparse.ArgumentParser(
        prog="akatsuki",
        description="Universal CLI and MCP Gateway for the akatsuki Agent Memory Substrate",
    )
    parser.add_argument(
        "--vault",
        "-V",
        help="Path to Akatsuki vault (defaults to $AKATSUKI_VAULT or auto-discovered)",
    )
    subparsers = parser.add_subparsers(dest="command")

    # init
    p_init = subparsers.add_parser("init", help="Bootstrap a new Akatsuki living memory vault")
    p_init.add_argument("path", nargs="?", default=".", help="Target directory to initialize vault in (default: .)")

    # search
    p_search = subparsers.add_parser("search", help="Search notes in akatsuki")
    p_search.add_argument("query", help="Keyword, topic, or exact phrase")
    p_search.add_argument(
        "--domain",
        "-d",
        choices=[
            "00-Meta",
            "01-Daily",
            "20-Projects",
            "30-Agents",
            "40-Systems",
            "50-Configs",
            "60-Scripts",
            "90-Reference",
            "90-Database",
        ],
    )
    p_search.add_argument("--limit", "-n", type=int, default=10)

    # read / cat
    p_read = subparsers.add_parser("read", aliases=["cat"], help="Read an akatsuki note or section")
    p_read.add_argument("note", help="Note title, stem, or relative path")
    p_read.add_argument("--section", "-s", help="Specific heading or section to extract")
    p_read.add_argument("--budget", "-b", type=int, help="Token budget packing (e.g. 200)")
    p_read.add_argument("--toc", action="store_true", help="Print table of contents outline")

    # contract
    p_contract = subparsers.add_parser("contract", help="Extract machine boundary contract from note")
    p_contract.add_argument("note", help="Note title, stem, or relative path")

    # get
    p_get = subparsers.add_parser("get", help="O(1) exact property getter")
    p_get.add_argument("keypath", help="Keypath (e.g. services.bountools.ports, entities.filament.repo)")

    # query
    p_query = subparsers.add_parser("query", help="Execute read-only SQL against SQLite index")
    p_query.add_argument("sql", help="SQL query string (SELECT ...)")

    # blast
    p_blast = subparsers.add_parser("blast", help="Calculate architectural blast radius")
    p_blast.add_argument("target", help="Component, service, or system name")

    # test
    p_test = subparsers.add_parser("test", help="Execute machine-verifiable assertion blocks")
    p_test.add_argument("note", nargs="?", help="Optional note to filter tests")

    # set
    p_set = subparsers.add_parser("set", help="Surgically update a frontmatter key-value property")
    p_set.add_argument("note", help="Note title, stem, or relative path")
    p_set.add_argument("--key", "-k", required=True, help="Dot-separated keypath (e.g. status)")
    p_set.add_argument("--value", "-v", required=True, help="New value as string or JSON")

    # lint
    subparsers.add_parser("lint", help="Validate vault notes against strict machine schemas")

    # append
    p_append = subparsers.add_parser("append", help="Append content under a specific heading in a note")
    p_append.add_argument("note", help="Note title, stem, or relative path")
    p_append.add_argument("--heading", "-H", required=True, help="Heading under which to append content")
    p_append.add_argument("--content", "-c", help="Markdown content string to append")
    p_append.add_argument("--file", "-f", help="File containing markdown to append (or stdin)")

    # list / ls
    p_list = subparsers.add_parser("list", aliases=["ls"], help="List notes in akatsuki")
    p_list.add_argument(
        "--domain",
        "-d",
        choices=[
            "00-Meta",
            "01-Daily",
            "20-Projects",
            "30-Agents",
            "40-Systems",
            "50-Configs",
            "60-Scripts",
            "90-Reference",
            "90-Database",
        ],
    )

    # write
    p_write = subparsers.add_parser("write", help="Write a note, config, or script to akatsuki")
    p_write.add_argument("path", help="Relative path inside vault (e.g. 20-Projects/app.md, 50-Configs/traefik.yml)")
    p_write.add_argument("--file", "-f", help="Source file to read content from (defaults to stdin)")
    p_write.add_argument("--overwrite", action="store_true", help="Overwrite if exists")
    p_write.add_argument("--raw", action="store_true", help="Write raw content without forcing .md extension or frontmatter enforcement")

    # services
    subparsers.add_parser("services", help="Print active services catalog and container allocations")

    # projects
    subparsers.add_parser("projects", help="Print active projects inventory")

    # daily
    p_daily = subparsers.add_parser("daily", help="Print today's or specified daily note")
    p_daily.add_argument("date", nargs="?", help="YYYY-MM-DD date (defaults to today)")

    # log
    p_log = subparsers.add_parser("log", help="Deposit a work log entry into today's note")
    p_log.add_argument("--project", "-p", default="", help="Project or repo name")
    p_log.add_argument("--summary", "-s", required=True, help="Summary of work or commit hash")
    p_log.add_argument("--device", "-d", default=None, help="Device/hostname identifier (defaults to current host)")

    # verify
    subparsers.add_parser("verify", help="Verify wikilinks integrity across vault")

    # mcp
    subparsers.add_parser("mcp", help="Run as Model Context Protocol (MCP) server on stdio")

    args = parser.parse_args()

    global CURRENT_VAULT_OVERRIDE
    if args.vault:
        CURRENT_VAULT_OVERRIDE = Path(args.vault).expanduser().resolve()

    if args.command == "init":
        cli_init(args)
    elif args.command == "search":
        cli_search(args)
    elif args.command in ("read", "cat"):
        cli_read(args)
    elif args.command == "contract":
        cli_contract(args)
    elif args.command == "get":
        cli_get(args)
    elif args.command == "query":
        cli_query(args)
    elif args.command == "blast":
        cli_blast(args)
    elif args.command == "test":
        cli_test(args)
    elif args.command == "set":
        cli_set(args)
    elif args.command == "lint":
        cli_lint(args)
    elif args.command == "append":
        cli_append(args)
    elif args.command in ("list", "ls"):
        cli_list(args)
    elif args.command == "write":
        cli_write(args)
    elif args.command == "services":
        cli_services(args)
    elif args.command == "projects":
        cli_projects(args)
    elif args.command == "daily":
        cli_daily(args)
    elif args.command == "log":
        cli_log(args)
    elif args.command == "verify":
        cli_verify(args)
    elif args.command == "mcp":
        run_mcp_server()
    else:
        parser.print_help()


if __name__ == "__main__":
    main()
