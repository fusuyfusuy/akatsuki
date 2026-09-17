"""Dense vector embedding, indexing, and retrieval for Akatsuki."""

import json
import math
import os
import re
import sqlite3
import struct
import subprocess
import sys
from pathlib import Path

from akatsuki.constants import DEFAULT_EMBED_BATCH_SIZE, DEFAULT_EMBED_MODEL

_EMBED_MODEL_INSTANCE = None


def normalize_vector(v: list[float]) -> list[float]:
    """L2 normalize float vector."""
    norm = math.sqrt(sum(x * x for x in v))
    if norm == 0.0:
        return v
    return [x / norm for x in v]


def dot_product(v1: list[float], v2: tuple[float, ...]) -> float:
    """Calculate inner product between two vectors."""
    return sum(a * b for a, b in zip(v1, v2, strict=False))


def pack_vector(v: list[float]) -> bytes:
    """Serialize float list into binary IEEE 754 float32 blob."""
    return struct.pack(f"{len(v)}f", *v)


def unpack_vector(blob: bytes, dim: int) -> tuple[float, ...]:
    """Deserialize binary float32 blob into float tuple."""
    return struct.unpack(f"{dim}f", blob)


def get_external_embed_python() -> Path | None:
    """Locate an external virtual environment with torch/sentence_transformers if available."""
    if os.environ.get("AKATSUKI_DISABLE_HOST_EMBED") == "1" or os.environ.get("AKATSUKI_TESTING") == "1":
        return None

    env_py = os.environ.get("AKATSUKI_EMBED_PYTHON")
    if env_py and Path(env_py).is_file():
        return Path(env_py)

    kb_env = os.environ.get("KNOWLEDGE_BASE_DIR")
    if kb_env and (Path(kb_env) / ".venv" / "bin" / "python").is_file():
        return Path(kb_env) / ".venv" / "bin" / "python"

    candidates = [
        Path.home() / "configs" / "knowledge-base" / ".venv" / "bin" / "python",
    ]
    for c in candidates:
        if c.is_file():
            return c
    return None


def get_embed_model(model_name: str = DEFAULT_EMBED_MODEL):
    """Lazy-load SentenceTransformer model on CPU clamped to 2 threads."""
    global _EMBED_MODEL_INSTANCE
    if _EMBED_MODEL_INSTANCE is not None:
        return _EMBED_MODEL_INSTANCE

    try:
        import torch
        from sentence_transformers import SentenceTransformer
    except ImportError as e:
        raise RuntimeError("SentenceTransformers/PyTorch not installed in this Python environment.") from e

    torch.set_num_threads(2)
    _EMBED_MODEL_INSTANCE = SentenceTransformer(model_name, device="cpu")
    return _EMBED_MODEL_INSTANCE


def encode_texts(
    texts: list[str],
    model_name: str = DEFAULT_EMBED_MODEL,
    batch_size: int = DEFAULT_EMBED_BATCH_SIZE,
) -> list[list[float]]:
    """Encode document chunks locally applying the E5 'passage: ' prefix."""
    try:
        model = get_embed_model(model_name)
        prefixed = [f"passage: {t}" if not t.startswith("passage: ") else t for t in texts]
        raw = model.encode(prefixed, batch_size=batch_size, normalize_embeddings=True, show_progress_bar=False)
        return [vec.tolist() for vec in raw]
    except RuntimeError:
        ext_py = get_external_embed_python()
        if ext_py and sys.executable != str(ext_py):
            code = (
                "from sentence_transformers import SentenceTransformer; "
                "import torch, json, sys; "
                "torch.set_num_threads(2); "
                f"m = SentenceTransformer({model_name!r}, device='cpu'); "
                "texts = json.loads(sys.stdin.read()); "
                "prefixed = [f'passage: {t}' for t in texts]; "
                "vecs = m.encode(prefixed, batch_size=32, normalize_embeddings=True, show_progress_bar=False); "
                "print(json.dumps([v.tolist() for v in vecs]))"
            )
            res = subprocess.run(
                [str(ext_py), "-c", code],
                input=json.dumps(texts),
                capture_output=True,
                text=True,
                check=True,
            )
            return json.loads(res.stdout)
        raise


def encode_query(query: str, model_name: str = DEFAULT_EMBED_MODEL) -> list[float]:
    """Encode search query applying the E5 'query: ' prefix."""
    try:
        model = get_embed_model(model_name)
        prefixed = f"query: {query.strip()}"
        raw = model.encode([prefixed], normalize_embeddings=True, show_progress_bar=False)
        return raw[0].tolist()
    except RuntimeError:
        ext_py = get_external_embed_python()
        if ext_py and sys.executable != str(ext_py):
            code = (
                "from sentence_transformers import SentenceTransformer; "
                "import torch, json; "
                "torch.set_num_threads(2); "
                f"m = SentenceTransformer({model_name!r}, device='cpu'); "
                f"q = {query.strip()!r}; "
                "vec = m.encode(['query: ' + q], normalize_embeddings=True, show_progress_bar=False); "
                "print(json.dumps(vec[0].tolist()))"
            )
            res = subprocess.run(
                [str(ext_py), "-c", code],
                capture_output=True,
                text=True,
                check=True,
            )
            return json.loads(res.stdout)
        raise


def get_vectors_db(vault: Path) -> sqlite3.Connection:
    """Connect to vault vectors SQLite database, creating schema if needed."""
    cache_dir = vault / ".akatsuki"
    cache_dir.mkdir(parents=True, exist_ok=True)
    db_path = cache_dir / "vectors.db"
    con = sqlite3.connect(str(db_path), timeout=30.0)
    con.row_factory = sqlite3.Row
    con.execute("PRAGMA journal_mode=WAL;")

    con.execute("""
        CREATE TABLE IF NOT EXISTS file_meta (
            rel_path TEXT PRIMARY KEY,
            mtime REAL NOT NULL,
            size INTEGER NOT NULL,
            chunk_count INTEGER NOT NULL,
            indexed_at TEXT NOT NULL
        );
    """)

    con.execute("""
        CREATE TABLE IF NOT EXISTS note_vectors (
            chunk_id TEXT PRIMARY KEY,
            rel_path TEXT NOT NULL,
            stem TEXT NOT NULL,
            domain TEXT NOT NULL,
            display_title TEXT NOT NULL,
            display_summary TEXT NOT NULL,
            tags TEXT,
            breadcrumb TEXT,
            chunk_index INTEGER NOT NULL,
            total_chunks INTEGER NOT NULL,
            preview TEXT NOT NULL,
            vector_blob BLOB NOT NULL,
            dim INTEGER NOT NULL
        );
    """)

    con.execute("CREATE INDEX IF NOT EXISTS idx_note_vectors_rel ON note_vectors(rel_path);")
    con.execute("CREATE INDEX IF NOT EXISTS idx_note_vectors_domain ON note_vectors(domain);")
    con.execute("CREATE INDEX IF NOT EXISTS idx_note_vectors_stem ON note_vectors(stem);")
    con.commit()
    return con


def chunk_akatsuki_note(
    vault: Path,
    rel_path: str,
    text: str,
    fm: dict,
    body: str,
) -> list[dict]:
    """Chunk note by markdown headings, falling back to sliding window."""
    stem = Path(rel_path).stem
    domain = rel_path.split("/")[0] if "/" in rel_path else ""
    display_title = str(fm.get("title") or stem)
    display_summary = str(fm.get("summary") or "")
    tags = fm.get("tags") or ""
    tags_str = " ".join(str(t) for t in tags) if isinstance(tags, list) else str(tags)

    doc_header = f"[Document: {display_title}]"
    if display_summary:
        doc_header += f"\n[Summary: {display_summary}]"
    if tags_str:
        doc_header += f"\n[Tags: {tags_str}]"

    heading_regex = re.compile(r"^(#{1,4})\s+(.+)$", re.MULTILINE)
    matches = list(heading_regex.finditer(body))

    sections: list[tuple[str, str]] = []
    if not matches:
        sections.append(("", body.strip()))
    else:
        first_start = matches[0].start()
        if first_start > 0:
            preamble = body[:first_start].strip()
            if preamble:
                sections.append(("Overview", preamble))

        for idx, m in enumerate(matches):
            h_title = m.group(2).strip()
            sec_start = m.end()
            sec_end = matches[idx + 1].start() if idx + 1 < len(matches) else len(body)
            sec_body = body[sec_start:sec_end].strip()
            sections.append((h_title, sec_body))

    raw_chunks: list[tuple[str, str]] = []
    for h_title, sec_body in sections:
        if not sec_body and not h_title:
            continue
        max_chunk_chars = 1200

        if len(sec_body) <= max_chunk_chars:
            raw_chunks.append((h_title, sec_body))
        else:
            paragraphs = sec_body.split("\n\n")
            cur_buf = []
            cur_len = 0
            for p in paragraphs:
                p_clean = p.strip()
                if not p_clean:
                    continue
                if cur_len + len(p_clean) > max_chunk_chars and cur_buf:
                    raw_chunks.append((h_title, "\n\n".join(cur_buf)))
                    cur_buf = [p_clean]
                    cur_len = len(p_clean)
                else:
                    cur_buf.append(p_clean)
                    cur_len += len(p_clean)
            if cur_buf:
                raw_chunks.append((h_title, "\n\n".join(cur_buf)))

    if not raw_chunks:
        raw_chunks.append(("", display_summary or display_title))

    chunks = []
    total_chunks = len(raw_chunks)
    for i, (bcrumb, sec_text) in enumerate(raw_chunks):
        chunk_id = f"{rel_path}:{i}"
        parts = [doc_header]
        if bcrumb:
            parts.append(f"[Breadcrumb: {bcrumb}]")
        if sec_text:
            parts.append(sec_text)
        embed_text = "\n".join(parts).strip()
        preview = sec_text[:280].strip() if sec_text else display_summary

        chunks.append(
            {
                "chunk_id": chunk_id,
                "rel_path": rel_path,
                "stem": stem,
                "domain": domain,
                "display_title": display_title,
                "display_summary": display_summary,
                "tags": tags_str,
                "breadcrumb": bcrumb,
                "chunk_index": i,
                "total_chunks": total_chunks,
                "preview": preview,
                "embed_text": embed_text,
            }
        )

    return chunks


def sync_vectors_index(
    vault: Path,
    con: sqlite3.Connection | None = None,
    model_name: str = DEFAULT_EMBED_MODEL,
) -> dict:
    """Incrementally synchronize SQLite vector database using file metadata (mtime, size)."""
    close_con = False
    if con is None:
        con = get_vectors_db(vault)
        close_con = True

    try:
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

        # 1. Prune deleted notes
        deleted = set(indexed.keys()) - set(current_files.keys())
        for d in deleted:
            con.execute("DELETE FROM note_vectors WHERE rel_path = ?", (d,))
            con.execute("DELETE FROM file_meta WHERE rel_path = ?", (d,))

        # 2. Check which notes are new or modified
        to_embed: list[tuple[str, Path, float, int]] = []
        for rel, (f, mtime, size) in current_files.items():
            prev = indexed.get(rel)
            if prev is not None and prev[0] == mtime and prev[1] == size:
                continue
            to_embed.append((rel, f, mtime, size))

        if not to_embed and not deleted:
            return {"added": 0, "updated": 0, "deleted": 0, "unchanged": len(current_files)}

        chunks_to_encode: list[dict] = []
        file_chunk_map: dict[str, tuple[float, int, int]] = {}

        from akatsuki.storage import parse_frontmatter

        for rel, f, mtime, size in to_embed:
            try:
                text = f.read_text(encoding="utf-8")
                fm, body = parse_frontmatter(text)
                note_chunks = chunk_akatsuki_note(vault, rel, text, fm, body)
                chunks_to_encode.extend(note_chunks)
                file_chunk_map[rel] = (mtime, size, len(note_chunks))
            except Exception:
                continue

        if chunks_to_encode:
            texts_to_embed = [c["embed_text"] for c in chunks_to_encode]
            embeddings = encode_texts(texts_to_embed, model_name=model_name)

            for c, vec in zip(chunks_to_encode, embeddings, strict=True):
                dim = len(vec)
                blob = pack_vector(vec)
                con.execute(
                    """INSERT OR REPLACE INTO note_vectors (
                        chunk_id, rel_path, stem, domain, display_title, display_summary,
                        tags, breadcrumb, chunk_index, total_chunks, preview, vector_blob, dim
                    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
                    (
                        c["chunk_id"],
                        c["rel_path"],
                        c["stem"],
                        c["domain"],
                        c["display_title"],
                        c["display_summary"],
                        c["tags"],
                        c["breadcrumb"],
                        c["chunk_index"],
                        c["total_chunks"],
                        c["preview"],
                        blob,
                        dim,
                    ),
                )

        import datetime

        now_iso = datetime.datetime.now().astimezone().isoformat(timespec="seconds")
        for rel, (mtime, size, count) in file_chunk_map.items():
            con.execute(
                """INSERT OR REPLACE INTO file_meta (rel_path, mtime, size, chunk_count, indexed_at)
                   VALUES (?, ?, ?, ?, ?)""",
                (rel, mtime, size, count, now_iso),
            )

        con.commit()
        added_count = sum(1 for rel in file_chunk_map if rel not in indexed)
        updated_count = len(file_chunk_map) - added_count
        return {
            "added": added_count,
            "updated": updated_count,
            "deleted": len(deleted),
            "unchanged": len(current_files) - len(file_chunk_map),
        }
    finally:
        if close_con:
            con.close()


def search_vectors_akatsuki(
    vault: Path,
    query: str,
    domain: str | None = None,
    limit: int = 10,
    con: sqlite3.Connection | None = None,
    model_name: str = DEFAULT_EMBED_MODEL,
) -> list[dict]:
    """Search Akatsuki vectors by cosine similarity, aggregating at document level."""
    close_con = False
    if con is None:
        con = get_vectors_db(vault)
        close_con = True

    try:
        sync_vectors_index(vault, con, model_name=model_name)
        q_vec = encode_query(query, model_name=model_name)

        sql = "SELECT chunk_id, rel_path, stem, domain, display_title, display_summary, breadcrumb, preview, vector_blob, dim FROM note_vectors"
        params: list[object] = []
        if domain:
            sql += " WHERE domain = ?"
            params.append(domain)

        cur = con.execute(sql, params)
        rows = cur.fetchall()
        if not rows:
            return []

        doc_best: dict[str, dict] = {}
        for r in rows:
            rel = r["rel_path"]
            dim = r["dim"]
            vec = unpack_vector(r["vector_blob"], dim)
            sim = dot_product(q_vec, vec)

            prev = doc_best.get(rel)
            if prev is None or sim > prev["similarity"]:
                doc_best[rel] = {
                    "rel_path": rel,
                    "stem": r["stem"],
                    "domain": r["domain"],
                    "title": r["display_title"],
                    "summary": r["display_summary"],
                    "similarity": sim,
                    "breadcrumb": r["breadcrumb"],
                    "snippet": r["preview"],
                    "vault": "akatsuki",
                }

        results = sorted(doc_best.values(), key=lambda x: x["similarity"], reverse=True)[:limit]
        for r in results:
            r["score"] = round(r["similarity"], 4)
            r["matches"] = [(1, r["snippet"])] if r["snippet"] else []
        return results
    finally:
        if close_con:
            con.close()
