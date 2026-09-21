"""Search engine, Okapi BM25 ranking, RRF hybrid fusion, and contract slicing for Akatsuki."""

import json
import re
from pathlib import Path

from akatsuki.constants import HAVE_PYYAML
from akatsuki.index import fts_db_context, sync_fts_index
from akatsuki.markdown import slice_markdown_section
from akatsuki.storage import parse_frontmatter, resolve_note_file
from akatsuki.vectors import search_vectors_akatsuki

if HAVE_PYYAML:
    import yaml


def expand_query_term(w: str) -> list[str]:
    """Generate morphological prefix/stem variations for FTS matching."""
    variants = [w]
    if len(w) > 4:
        variants.append(f"{w}*")
        for suffix in ("ing", "ed", "es", "s", "er", "able", "ive", "tion", "ment"):
            if w.endswith(suffix) and len(w) - len(suffix) >= 3:
                stem = w[: -len(suffix)]
                variants.append(f"{stem}*")
                variants.append(stem)
                break
    return list(dict.fromkeys(variants))


def build_fts_clause(words: list[str], op: str = "AND") -> str:
    """Build FTS5 search expression with term expansion."""
    clauses = []
    for w in words:
        exp = expand_query_term(w.lower())
        if len(exp) == 1:
            clauses.append(exp[0])
        else:
            sub = " OR ".join(exp)
            clauses.append(f"({sub})")
    return f" {op} ".join(clauses)


def extract_note_contract(vault: Path, note_query: str, as_json: bool = False) -> tuple[str, bool]:
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
            invariants = [ln.strip()[2:].strip() for ln in inv_slice.splitlines() if ln.strip().startswith("- ")]

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

    if as_json or not HAVE_PYYAML:
        return json.dumps(clean_data, indent=2, default=str), False
    return yaml.dump(clean_data, sort_keys=False).strip(), False


def get_keypath(vault: Path, keypath: str) -> tuple[str, bool]:
    """Retrieve exact property or entity at keypath."""
    parts = [p.strip() for p in keypath.split(".") if p.strip()]
    if not parts:
        return "Error: Empty keypath.", True

    category = parts[0]
    if category in ("services", "entities") and len(parts) >= 2:
        with fts_db_context(vault) as con:
            sync_fts_index(vault, con)

            if category == "services":
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

            elif category == "entities":
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


def search_vault(
    vault: Path,
    query: str,
    domain: str | None = None,
    limit: int | str = 10,
    with_graph: bool = False,
    mode: str = "hybrid",
) -> list[dict]:
    """Search notes in Akatsuki vault.
    Modes:
      - 'hybrid': BM25 + dense semantic vectors via Reciprocal Rank Fusion (RRF, k=60, default).
      - 'bm25': Pure lexical Okapi BM25 ranking over SQLite FTS5 index.
      - 'vector': Pure semantic dense vector retrieval over multilingual-e5-small (384D).
    """
    clean_query = query.strip()
    if not clean_query:
        return []

    if limit is not None:
        try:
            limit = int(limit)
        except (ValueError, TypeError):
            limit = 10
    else:
        limit = 10

    def _run_akatsuki_bm25(cand_limit: int) -> list[dict]:
        with fts_db_context(vault) as con:
            sync_fts_index(vault, con)

            words = re.findall(r"\w+", clean_query)
            if not words:
                return []

            if clean_query.startswith('"') and clean_query.endswith('"') and len(clean_query) > 2:
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
                params = (q_candidate, domain, cand_limit) if domain else (q_candidate, cand_limit)
                try:
                    cur = con.execute(sql, params)
                    rows = cur.fetchall()
                    if rows:
                        break
                except Exception:
                    continue

            ak_results = []
            for r in rows:
                snip = r["snippet"].strip() if r["snippet"] else ""
                ak_results.append(
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
            return ak_results

    # Mode: bm25
    if mode == "bm25":
        results = _run_akatsuki_bm25(limit)

    # Mode: vector
    elif mode == "vector":
        try:
            vec_hits = search_vectors_akatsuki(vault, clean_query, domain=domain, limit=limit)
        except Exception:
            vec_hits = []
        results = sorted(vec_hits, key=lambda x: x.get("score", 0.0), reverse=True)[:limit]

    # Mode: hybrid (default)
    else:
        cand_limit = max(limit * 2, 20)
        bm25_hits = _run_akatsuki_bm25(cand_limit)
        try:
            vec_hits = search_vectors_akatsuki(vault, clean_query, domain=domain, limit=cand_limit)
        except Exception:
            vec_hits = []

        if not vec_hits:
            results = bm25_hits[:limit]
        elif not bm25_hits:
            results = vec_hits[:limit]
        else:
            k = 60.0
            rrf_scores: dict[str, float] = {}
            doc_map: dict[str, dict] = {}

            for rank, h in enumerate(bm25_hits, 1):
                key = h["rel_path"]
                rrf_scores[key] = rrf_scores.get(key, 0.0) + 1.0 / (k + rank)
                doc_map[key] = dict(h)

            for rank, h in enumerate(vec_hits, 1):
                key = h["rel_path"]
                rrf_scores[key] = rrf_scores.get(key, 0.0) + 1.0 / (k + rank)
                if key not in doc_map:
                    doc_map[key] = dict(h)
                else:
                    if h.get("snippet") and not doc_map[key].get("snippet"):
                        doc_map[key]["snippet"] = h["snippet"]
                    if h.get("breadcrumb"):
                        doc_map[key]["breadcrumb"] = h["breadcrumb"]

            sorted_keys = sorted(rrf_scores.keys(), key=lambda kd: rrf_scores[kd], reverse=True)[:limit]
            results = []
            for sk in sorted_keys:
                item = doc_map[sk]
                item["score"] = round(rrf_scores[sk], 4)
                results.append(item)

    if with_graph and results:
        with fts_db_context(vault) as con:
            for item in results:
                s_stem = item["stem"]
                cur_up = con.execute(
                    "SELECT source_rel, relation_type FROM relations WHERE target_stem = ? LIMIT 5",
                    (s_stem,),
                )
                up_rows = [f"{row['source_rel']} ({row['relation_type']})" for row in cur_up.fetchall()]

                rel_path = item["rel_path"]
                cur_down = con.execute(
                    "SELECT target_stem, relation_type FROM relations WHERE source_rel = ? LIMIT 5",
                    (rel_path,),
                )
                down_rows = [f"{row['target_stem']} ({row['relation_type']})" for row in cur_down.fetchall()]

                cur_svc = con.execute(
                    "SELECT name, ports FROM services WHERE name = ? OR container_prefix = ? OR rel_path = ? LIMIT 3",
                    (s_stem, s_stem, rel_path),
                )
                svc_rows = [
                    f"{row['name']}" + (f":{row['ports']}" if row["ports"] else "") for row in cur_svc.fetchall()
                ]

                item["graph"] = {
                    "upstream": up_rows,
                    "downstream": down_rows,
                    "services": svc_rows,
                }

    return results
