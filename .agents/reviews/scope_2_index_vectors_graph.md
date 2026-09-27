---
scope: "Scope 2: Indexing, Vectors & Knowledge Graph Engine"
score: 8.8
status: "MINOR"
critical_findings: 0
invariant_breaches:
  - "Missing models mark file_meta indexed: note_vectors remains empty after setup-models [src/index/mod.rs:693]"
  - "Unreadable files silently deleted from SQLite projection via filter_map [src/index/mod.rs:306]"
  - "PageRank centrality engine absent from knowledge graph [src/graph/mod.rs:319]"
---

# Scope 2 Audit: Indexing, Vectors & Knowledge Graph Engine

## 1. Executive Summary & Scorecard
- **Overall Score**: **8.8 / 10.0** (Status: **MINOR**)
- **Target Files Audited**:
  - `src/index/mod.rs` (711 lines)
  - `src/vectors/mod.rs` (646 lines)
  - `src/graph/mod.rs` (570 lines)
  - `tests/regression_tests.rs` (603 lines)
- **Verdict**: The indexing and knowledge graph subsystem demonstrates robust SQLite WAL operation, clean 100% parameterized SQL bindings, parallel Blake3 Merkle scanning via Rayon, and safe BERT 512-token truncation for Candle embeddings. Prior defects (malformed YAML frontmatter aborts, wikilink path stem extraction, BERT position panics) have been resolved and verified via 21 regression tests. Remaining issues center on vector synchronization on delayed model provisioning, silent note deletion on read errors, and absence of PageRank.

| Dimension | Score | Status | Key Drivers |
|---|---|---|---|
| **Correctness** | 8.3 | Moderate | PageRank absent (DFS-only); vault root notes missed in boundary sinks; duplicate relation edges; code fence comments chunked as headings. |
| **Robustness** | 8.4 | Moderate | Missing vector models record `file_meta`, leaving vectors stale post-setup; `fs::read_to_string` error triggers silent index deletion; 3-tx split desync. |
| **Performance** | 8.7 | Minor | Fast Rayon Blake3 scan; $B=1$ sequential embedding bottleneck; `LIKE '%/...'` full table scans in graph hops; unindexed vector table scans. |
| **Security** | 9.9 | Exemplary | 100% parameterized queries via `rusqlite::params![]`; safe `f32::from_le_bytes` deserialization; robust vault path isolation. |

---

## 2. Review Dimensions & Detailed Findings

### A. Correctness
1. **Absence of PageRank Centrality ([src/graph/mod.rs:319-400](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/graph/mod.rs#L319-L400))**:
   - The graph engine contains no PageRank implementation (no damping factor, power iteration, convergence threshold, or dead-end/sink redistribution). All graph mappings are depth-clamped (1..=5) DFS tree traversals (`traverse_down`, `traverse_up`).
2. **Vault Root Boundary Sink Omission ([src/graph/mod.rs:110, 481](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/graph/mod.rs#L110))**:
   - Boundary sink queries filter via `rel_path LIKE '%/{stem}.md'`. Root notes (e.g., `Services.md`) lack a leading slash (`/`) and are omitted when `name != stem` and `container_prefix != stem`.
3. **Heading Regex in Vector Chunker Splits on Code Comments ([src/vectors/mod.rs:132-153](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/vectors/mod.rs#L132-L153))**:
   - `Regex::new(r"(?m)^(#{1,4})[ \t]+(.+)$")` operates across the raw body without tracking code fences (` ``` `), unlike `storage::extract_section`. Shell/Python comments inside scripts are falsely parsed as heading boundaries and breadcrumbs.
4. **Duplicate Relation Edges Multiply Traversal Branches ([src/index/mod.rs:199-203, 555-560](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/index/mod.rs#L199-L203))**:
   - `relations` table lacks a `UNIQUE(source_rel, target_stem, relation_type)` constraint. Multiple wikilinks to the same note insert duplicate rows, causing `traverse_down` and `blast_radius_with` to expand duplicate subtrees.
5. **BERT Position Clamping & Cosine Similarity ([src/vectors/mod.rs:396-423, 608-613](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/vectors/mod.rs#L396-L423))**:
   - Compliant: `len = ids.len().min(512)` strictly bounds tokens to position embeddings; Candle mean pooling with $1e-9$ clamp and L2-norm enables accurate dot-product cosine retrieval.

### B. Robustness
1. **Permanent Vector Starvation on Delayed Model Setup ([src/index/mod.rs:689-698](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/index/mod.rs#L689-L698), [src/vectors/mod.rs:449-454](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/vectors/mod.rs#L449-L454))**:
   - When model weights are absent, `sync_note_vectors` returns `Ok("skipped ...")`. `meta_tx` then commits the latest Blake3 hashes to `file_meta`.
   - After the user runs `akatsuki setup-models`, subsequent `reconcile` runs find all hashes matching in `file_meta` (`unchanged`), never generating embeddings. `note_vectors` remains permanently empty until files are touched or cache is wiped.
2. **Silent Note Deletion on File Read Error ([src/index/mod.rs:306, 326-330](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/index/mod.rs#L306))**:
   - `fs::read_to_string(abs).ok()?` silently filters out unreadable notes. The diff calculator interprets missing paths as deletions, executing destructive `DELETE` across FTS, entities, and vectors with zero error logged.
3. **Transaction Split Desync ([src/index/mod.rs:687-699](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/index/mod.rs#L687-L699))**:
   - `tx` commits entities and FTS before `sync_note_vectors` runs. If vector embedding fails midway (OOM, device failure), `notes_fts` reflects new data while `note_vectors` retains stale data, and `file_meta` retains old hashes.
4. **Frontmatter Isolation ([src/index/mod.rs:437-444](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/index/mod.rs#L437-L444), [src/vectors/mod.rs:83-92](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/vectors/mod.rs#L83-L92))**:
   - Frontmatter YAML parse errors are cleanly isolated in both FTS indexing and vector chunking without aborting the pipeline.

### C. Performance
1. **Sequential $B=1$ Vector Embedding ([src/vectors/mod.rs:460-498](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/vectors/mod.rs#L460-L498))**:
   - Chunks are embedded individually with batch size 1, underutilizing SIMD and multicore tensor throughput.
2. **Unindexed Graph Traversal LIKE Queries ([src/graph/mod.rs:98-100, 334-336](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/graph/mod.rs#L98-L100))**:
   - `WHERE source_rel = ?1 OR source_rel LIKE ?2 OR source_rel LIKE ?3` forces full table scans of `relations` on every hop due to `%` prefix in `nested_rel`.
3. **Full In-Memory Vector Scans ([src/vectors/mod.rs:541-582](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/vectors/mod.rs#L541-L582))**:
   - `search_vectors` queries all blobs into memory for flat Rust-side dot-product calculation.

### D. Security
1. **Parameterized Queries**: 100% of SQLite operations across `index`, `vectors`, and `graph` use `params![]`.
2. **Deserialization Safety**: `unpack_vector` uses `as_chunks::<4>()` and `f32::from_le_bytes` with strict dimension checks (`dim * 4 == blob.len()`).
3. **Path Containment**: File paths are strictly stripped against vault root ([src/index/mod.rs:305](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/index/mod.rs#L305)); model directories are hardcoded to user cache.

---

## 3. Regression Test Coverage Audit
- **Tests Evaluated** (`tests/regression_tests.rs`):
  - `long_note_chunks_do_not_exceed_bert_position_embeddings` (L164): Verifies BERT 512-token clamping.
  - `chunk_note_succeeds_on_malformed_frontmatter` (L277): Verifies vector chunking frontmatter isolation.
  - `wikilink_directory_paths_resolve_to_stem` (L381): Verifies stem extraction for wikilinks and declared relations.
  - `audit_links_handles_duplicate_stems_across_domains` (L431): Verifies multi-map duplicate stem tracking.
  - `map_invalid_direction_exits_with_code_2` (L492): Verifies CLI argument validation.
  - `mcp_parameter_aliasing_contract_blast_map` (L512): Verifies MCP parameter aliasing.
- All 21 regression tests pass cleanly under both default and `--features vectors` configurations.

---

## 4. Prioritized Actionable Remediations

| Priority | File & Line | Issue | Remediation |
|---|---|---|---|
| **P0** | `src/index/mod.rs:693` | Empty vectors after `setup-models` | Only insert `file_meta` if vectors are embedded, or detect missing vectors during reconcile. |
| **P1** | `src/index/mod.rs:306` | Silent deletion on read error | Record read errors in `parse_errors` and retain existing `file_meta` rather than deleting. |
| **P1** | `src/graph/mod.rs:110` | Root note sink omission | Update sink query to `rel_path = ?1 OR rel_path LIKE ?2` to capture root notes without leading slash. |
| **P2** | `src/vectors/mod.rs:132` | Code fence heading chunking | Skip code fences in vector chunking regex to avoid splitting on code comments. |
| **P2** | `src/index/mod.rs:200` | Duplicate relation rows | Add `UNIQUE(source_rel, target_stem, relation_type)` to `relations` schema and use `INSERT OR IGNORE`. |
| **P2** | `src/graph/mod.rs:334` | Full table scan in traversal | Resolve `stem` to exact `rel_path` via `entities` index before querying `relations`. |
| **P3** | `src/graph/mod.rs:319` | PageRank algorithm absent | Implement standard PageRank with 0.85 damping factor and dead-end redistribution. |
