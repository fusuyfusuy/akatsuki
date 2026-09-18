---
scope: "index-vectors-graph"
score: 7.8
status: "MODERATE"
critical_findings: 2
invariant_breaches:
  - "Orphan vector chunks persist across note truncations (data corruption in similarity rankings)"
  - "Boundary sink calculation drops dependencies beyond depth 1 in graph traversal"
---

# Scope 2 Audit: Index, Vectors & Graph Subsystems

## 1. Executive Summary & Scorecard
- **Overall Health Score**: **7.8 / 10.0** (Status: **MODERATE**)
- **Target Files**:
  - `src/akatsuki/index.py` (339 lines)
  - `src/akatsuki/vectors.py` (459 lines)
  - `src/akatsuki/graph.py` (242 lines)
  - `tests/test_map_and_search.py` (240 lines)
- **Summary**: SQLite FTS5 indexing and relational metadata extraction are structurally sound, and graph traversal cleanly detects cycles. However, the vector pipeline has severe defects: `sqlite-vec` is completely absent (forcing an in-memory full-table scan on every query), orphan chunks linger permanently on note truncation, and external worker subprocesses lack timeouts. In addition, `vectors.py` has zero unit test coverage.

| Dimension | Score | Status | Key Drivers |
|---|---|---|---|
| **Correctness** | 7.5 | Moderate | Stale chunk retention on edits; wikilink section anchor corruption; depth>=2 boundary sinks dropped. |
| **Robustness** | 7.6 | Moderate | Indefinite subprocess hang risk (no timeout); no auto-healing on corrupt index databases. |
| **Performance** | 7.2 | Moderate | Complete absence of `sqlite-vec` (pure Python table scan); unbatched all-vault embedding accumulation. |
| **Security** | 8.8 | Minor | Parameterized SQL throughout; code string interpolation in `encode_query`; regex false positives in read-only SQL check. |
| **Test Quality** | 7.9 | Moderate | Good graph and search dispatch tests; zero direct unit test coverage for `vectors.py`. |

---

## 2. Key Findings & Vulnerabilities

### A. Correctness
1. **Orphan Vector Chunks on Modification (`src/akatsuki/vectors.py:356-375`)**:
   - `sync_vectors_index` issues `INSERT OR REPLACE INTO note_vectors` keyed by `f"{rel}:{i}"`.
   - When a note is shortened (e.g. 8 chunks to 3), chunks 3–7 are never deleted because `DELETE FROM note_vectors WHERE rel_path = ?` only runs for completely deleted files (line 319). Obsolete chunks linger permanently, skewing vector similarity.
2. **Wikilink Anchor Corruption (`src/akatsuki/index.py:229-234`)**:
   - `re.findall(r"(?<!\\)\[\[([^\]\|]+)(?:\|[^\]]+)?\]\]", body)` captures section links like `[[Note#Section]]`.
   - `Path("Note#Section").stem` evaluates to `"Note#Section"`. `relations.target_stem` is saved with `#Section`, breaking blast-radius and upstream/downstream lookups against `Note`.
3. **Truncated Boundary Sink Discovery in Deep Graph (`src/akatsuki/graph.py:186-200`)**:
   - `all_stems` is populated by iterating only top-level children: `for c in downstream_tree: all_stems.add(c["stem"])`.
   - Grandchildren and deeper nodes (`c["children"]`) are ignored. Boundary sinks for dependencies at depth >= 2 are silently omitted.
4. **Unescaped SQL LIKE Wildcards (`src/akatsuki/graph.py:26-28, 133-135`)**:
   - Queries use `source_rel LIKE f"%/{t_stem}.md"` without `ESCAPE`. Stems containing underscores (e.g. `node_a`) treat `_` as a single-character wildcard, matching unrelated paths (`node-a.md`, `nodexa.md`).

### B. Robustness & Subprocess Management
1. **Subprocess Hang & Masked Diagnostics (`src/akatsuki/vectors.py:104-110, 134-139`)**:
   - `subprocess.run([str(ext_py), "-c", code], ...)` lacks a `timeout`. If PyTorch hangs, deadlocks, or hangs downloading model weights, the process blocks indefinitely.
   - On error, `check=True` raises `CalledProcessError`, masking `res.stderr` from logs unless explicitly caught.
2. **Missing Corrupt SQLite File Recovery (`src/akatsuki/index.py:18-20`, `src/akatsuki/vectors.py:149-151`)**:
   - Database connections fail catastrophically if `index.db` or `vectors.db` suffers disk corruption. Since markdown files are the authoritative source of truth, there is no automatic fallback to wipe and rebuild.
3. **Search Triggers Write-Sync Contention (`src/akatsuki/vectors.py:416`)**:
   - `search_vectors_akatsuki` invokes `sync_vectors_index` on every query, acquiring write locks even during read-only search operations.

### C. Performance & Architectural Omissions
1. **Missing `sqlite-vec` Extension Integration (`src/akatsuki/vectors.py:419-451`)**:
   - Despite specification requirements, `sqlite-vec` (or `vec0` virtual tables) is completely absent.
   - Every search queries the entire `note_vectors` table, deserializes every vector blob in pure Python (`struct.unpack`), and computes cosine similarity in a Python loop ($O(N \cdot D)$ overhead).
2. **Unbatched Full-Vault Memory Accumulation (`src/akatsuki/vectors.py:333-350`)**:
   - `sync_vectors_index` accumulates all chunks from all modified files into `chunks_to_encode`, serializes them into a single monolithic JSON payload over stdin, and encodes them in one pass. Full-vault indexing risks OOM.
3. **Hardcoded Batch Size in External Worker (`src/akatsuki/vectors.py:101`)**:
   - `encode_texts` accepts a `batch_size` parameter, but hardcodes `batch_size=32` in the external worker code string.
4. **Absence of PageRank Algorithm (`src/akatsuki/graph.py`)**:
   - The graph pipeline relies exclusively on recursive tree rendering; PageRank / graph centrality is not implemented.

### D. Security
1. **Code String Formatting in Subprocess (`src/akatsuki/vectors.py:130`)**:
   - `encode_query` injects `f"q = {query.strip()!r};"` directly into a `-c` Python script string. Passing inputs via `sys.stdin` (as done in `encode_texts`) is safer and avoids syntax errors on unusual Unicode/null characters.
2. **False Positives in Read-Only SQL Sanitizer (`src/akatsuki/index.py:313-315`)**:
   - Naive regex `rf"\b{forbidden}\b"` rejects harmless queries like `SELECT * FROM notes WHERE summary LIKE '%update%'`.

---

## 3. Prioritized Actionable Remediations

| Priority | Component | Remediation |
|---|---|---|
| **P0 (Critical)** | `vectors.py` | Add `con.execute("DELETE FROM note_vectors WHERE rel_path = ?", (rel,))` prior to inserting new chunks in `sync_vectors_index`. |
| **P0 (Critical)** | `vectors.py` | Add `timeout=120.0` to `subprocess.run` calls in `encode_texts` and `encode_query`, with `try...except TimeoutExpired`. |
| **P1 (High)** | `graph.py` | Recursively flatten all descendants in `traverse_graph` when compiling `all_stems` for `boundary_sinks`. |
| **P1 (High)** | `index.py` | Strip wikilink anchors: `m.split("#")[0].strip()` before deriving stem in regex link extractor. |
| **P1 (High)** | `vectors.py` | Add unit test suite for `vectors.py` (chunking, vector serialization, similarity, and index sync). |
| **P2 (Medium)** | `vectors.py` | Implement `sqlite-vec` extension loading with KNN index, preserving pure-Python dot product as fallback. |
| **P2 (Medium)** | `vectors.py` | Chunk `chunks_to_encode` into bounded batches (e.g. 128 chunks) during full-vault indexing. |
| **P3 (Low)** | `graph.py` | Add `ESCAPE` clause to `LIKE` queries matching file paths with underscores. |
