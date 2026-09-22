---
scope: "index_vectors_graph"
score: 8.5
status: "MINOR"
critical_findings: 0
invariant_breaches:
  - "file_meta and note_vectors dual-tx desync on vector failure [src/index/mod.rs#L661]"
  - "chunk_note unparseable frontmatter error leak aborts vector sync [src/vectors/mod.rs#L65]"
---

# Scope 2 Audit: Index, Vectors & Graph Subsystems (Native Rust v0.2.0)

## 1. Executive Summary & Scorecard
- **Health Score**: **8.5 / 10.0** (Status: **MINOR**)
- **Target Files**:
  - [`src/index/mod.rs`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs) (676 lines)
  - [`src/vectors/mod.rs`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/vectors/mod.rs) (621 lines)
  - [`src/graph/mod.rs`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/graph/mod.rs) (570 lines)
  - [`src/search/mod.rs`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/search/mod.rs) (`fuse_rrf` boundary)
  - [`tests/regression_tests.rs`](file:///home/devhax/projects/fusuyfusuy/akatsuki/tests/regression_tests.rs) (179 lines)
  - [`.agents/memory.md`](file:///home/devhax/projects/fusuyfusuy/akatsuki/.agents/memory.md)
- **Summary**: The Rust rewrite replaces the legacy Python subprocess pipeline with high-performance in-process Candle neural embeddings, parallel Blake3 hashing, and robust SQLite WAL FTS5 projection. All 7 regression tests pass. Two significant architectural defects exist: (1) `file_meta` is committed before `sync_note_vectors`, causing state desync on embedding failure, and (2) `chunk_note` propagates frontmatter YAML errors rather than isolating them, causing full reconcile aborts on malformed notes. PageRank is absent; graph queries rely solely on DFS tree traversal.

| Dimension | Score | Status | Key Drivers |
|---|---|---|---|
| **Correctness** | 8.4 | Moderate | Dual-tx vector desync; wikilink subpath stems unnormalized; PageRank algorithm absent (DFS-only). |
| **Robustness** | 8.2 | Moderate | Unparseable YAML aborts vector sync; read errors silently delete indexed notes; no corrupt DB auto-recovery. |
| **Performance** | 8.6 | Minor | Fast Rayon Blake3 scan; sequential vector embedding ($B=1$); unindexed `LIKE '%/...'` scans in graph hops. |
| **Security** | 9.8 | Exemplary | 100% parameterized SQL bindings via `rusqlite::params![]`; zero dynamic SQL injection surface. |

---

## 2. Review Dimensions & Detailed Findings

### A. Correctness
1. **Transaction Split Desync ([src/index/mod.rs#L661-L663](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L661-L663))**:
   - `tx.commit()?` commits `file_meta`, `notes_fts`, and `entities` before `sync_note_vectors(con, &vector_sources)?` executes.
   - If vector embedding fails (OOM, missing model mid-run, thread kill), `file_meta` retains the updated Blake3 hash. Subsequent syncs mark the note `unchanged`, leaving `note_vectors` permanently stale or missing.
2. **Wikilink Path Stem Misresolution ([src/index/mod.rs#L520-L527](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L520-L527))**:
   - `clean_stem` only strips `.md` (`target_stem.strip_suffix(".md")`), preserving directory prefixes (e.g. `[[40-Systems/Database]]` -> `"40-Systems/Database"`).
   - In [`src/index/mod.rs#L386-L389`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L386-L389), `entities.stem` is strictly the filename stem (`"Database"`). Backlink and blast radius queries (`WHERE target_stem = ?1`) fail to match path-qualified wikilinks.
   - Frontmatter declared relations ([src/index/mod.rs#L484-L509](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L484-L509)) do not strip `.md` or `[[...]]`.
3. **Absence of PageRank Centrality ([src/graph/mod.rs#L319-L400](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/graph/mod.rs#L319-L400))**:
   - The graph engine contains no PageRank implementation (damping 0.85, convergence thresholds). Graph queries are strictly depth-clamped (1..=5) DFS tree traversals (`traverse_down` / `traverse_up`).
4. **Candle Neural Pipeline & BERT Clamping ([src/vectors/mod.rs#L365-L398](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/vectors/mod.rs#L365-L398))**:
   - Verified compliant: 512-token BERT clamping prevents Candle tensor panics (tested via [`tests/regression_tests.rs#L164-L178`](file:///home/devhax/projects/fusuyfusuy/akatsuki/tests/regression_tests.rs#L164-L178)); mean pooling with epsilon clamping ($1e-9$) and L2 normalization accurately enables unit dot-product cosine ranking.
5. **RRF Rank Fusion ($k=60$) ([src/search/mod.rs#L217-L247](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/search/mod.rs#L217-L247))**:
   - Correctly combines BM25 and vector hits using $1.0 / (60.0 + \text{rank})$. Lacks deterministic tie-breaker sorting on equal scores.
6. **Root Boundary Sink Omission ([src/graph/mod.rs#L110-L113](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/graph/mod.rs#L110-L113))**:
   - Boundary sinks use `rel_path LIKE '%/{stem}.md'`. Notes residing in the vault root (`Services.md`) lack a leading slash and are missed when `name != stem`.

### B. Robustness
1. **Frontmatter Parse Isolation Leak in Vectors ([src/vectors/mod.rs#L64-L67](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/vectors/mod.rs#L64-L67), [src/vectors/mod.rs#L436](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/vectors/mod.rs#L436))**:
   - [`src/index/mod.rs#L398-L401`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L398-L401) isolates frontmatter YAML errors into `SyncReport.parse_errors`. However, `sync_note_vectors` calls `chunk_note`, which invokes `parse_frontmatter(content)?`. An unparseable note causes the entire sync to fail with an unhandled `Err`.
2. **Silent Note Deletion on File Read Error ([src/index/mod.rs#L263-L287](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L263-L287))**:
   - `fs::read_to_string(abs).ok()?` silently drops unreadable files (permissions, non-UTF8) from `current_map`. The diff engine classifies them as deleted and removes their records from SQLite.
3. **Corrupted `cache.db` Handling ([src/index/mod.rs#L48-L61](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L48-L61))**:
   - Missing DB auto-initializes via WAL; but corrupted DB headers fail hard without automated wipe-and-rebuild.
4. **Vector Degradation ([src/vectors/mod.rs#L221-L235](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/vectors/mod.rs#L221-L235))**:
   - Graceful degradation without weights; non-blocking fallback to BM25 with explicit warning notices.

### C. Performance
1. **Sequential Embedding Bottleneck ([src/vectors/mod.rs#L442-L444](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/vectors/mod.rs#L442-L444))**:
   - Chunks are embedded individually ($B=1$) with repeated forward passes, underutilizing SIMD and multicore parallelism.
2. **Full Table Scans on Graph Traversal ([src/graph/mod.rs#L97-L100](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/graph/mod.rs#L97-L100), [src/graph/mod.rs#L333-L336](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/graph/mod.rs#L333-L336))**:
   - `WHERE source_rel = ?1 OR source_rel LIKE ?2 OR source_rel LIKE ?3` cannot utilize `idx_relations_source` due to leading `%` in `?2` and `?3`, forcing full table scans on every traversal hop.
3. **Full Memory Buffering During Vault Scan ([src/index/mod.rs#L260-L280](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L260-L280))**:
   - Rayon scan buffers complete file content strings for all notes in RAM, even when 99% are unchanged.
4. **Relations Edge Duplication ([src/index/mod.rs#L523-L526](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L523-L526))**:
   - Multiple identical wikilinks insert redundant edges, multiplying graph tree expansion and sink queries.

### D. Security
1. **SQL Injection Resistance**: 100% of SQLite operations across `index`, `vectors`, and `graph` utilize parameterized queries (`params![]`).
2. **Path Containment & Frontmatter Isolation**: Frontmatter isolation is enforced on storage mutations ([`tests/regression_tests.rs#L56-L72`](file:///home/devhax/projects/fusuyfusuy/akatsuki/tests/regression_tests.rs#L56-L72)).

---

## 3. Prioritized Actionable Remediations

| Priority | Component | Exact Remediation |
|---|---|---|
| **P0 (Critical)** | `src/vectors/mod.rs` | Update [`chunk_note`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/vectors/mod.rs#L64) to gracefully handle invalid frontmatter by defaulting to `({})` and body, preventing sync aborts. |
| **P0 (Critical)** | `src/index/mod.rs` | Defer committing `file_meta` until `sync_note_vectors` succeeds ([src/index/mod.rs#L661](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L661)), or wrap both under an atomic transaction. |
| **P1 (High)** | `src/index/mod.rs` | Record I/O read failures in `SyncReport.parse_errors` instead of silently dropping files ([src/index/mod.rs#L263](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L263)). |
| **P1 (High)** | `src/index/mod.rs` | Extract true file stem in wikilinks (`Path::new(target_stem).file_stem()`) and strip `.md`/`[[]]` in declared relations ([src/index/mod.rs#L521](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L521)). |
| **P2 (Medium)** | `src/graph/mod.rs` | Resolve `stem` to `rel_path` prior to graph traversal queries ([src/graph/mod.rs#L333](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/graph/mod.rs#L333)) to eliminate unindexed `LIKE '%/...'` table scans. |
| **P2 (Medium)** | `src/vectors/mod.rs` | Implement batched tensor forward passes ($B=16$ or $32$) in `CandleEmbedder` for passage encoding ([src/vectors/mod.rs#L442](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/vectors/mod.rs#L442)). |
| **P3 (Low)** | `src/index/mod.rs` | Add `UNIQUE(source_rel, target_stem, relation_type)` on `relations` table ([src/index/mod.rs#L156](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L156)) to prevent duplicate graph edges. |
