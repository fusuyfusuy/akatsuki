---
scope: "domain_queries_mutations"
score: 8.5
status: "MINOR"
critical_findings: 0
invariant_breaches: []
---

# Scope 3 Audit: Domain Queries, Mutations & Invariants Engine

## 1. Executive Summary & Health Score
- **Overall Score**: 8.5 / 10 (`MINOR`)
- **Primary Strengths**: Process-safe advisory locking (`VaultLock`) with strict non-reentrancy separation (`append_work_log` vs `append_work_log_inner`); atomic file replacement (`write_atomic`); indentation-preserving YAML scalar auto-quoting (`quote_colon_scalars`); strict path containment and traversal blocking (`contained_path`); process group isolation with hard ceiling timeout and `SIGKILL` on invariant assertions.
- **Key Vulnerabilities**: Mutating CTE bypass in SQL query validator; section locator (`locate_section`) splitting on comments inside code blocks; stem shadowing across domains in link audits; keypath traversal failing on JSON arrays; pipe buffer exhaustion risking false timeouts on verbose invariant commands.

## 2. Findings Matrix

| Ref | Severity | File:Line | Category | Summary |
|---|---|---|---|---|
| F-01 | HIGH | [src/search/mod.rs#L357-L364](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/search/mod.rs#L357-L364) | Robustness | `execute_sql_query` only checks initial keyword against `SELECT\|WITH\|EXPLAIN`, allowing mutating CTEs (`WITH ... INSERT/DELETE`) and rejecting leading comments. |
| F-02 | MEDIUM | [src/storage/mod.rs#L447-L485](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L447-L485) | Robustness | `locate_section` does not track code fences; `# comment` lines inside code blocks match as H1 headings, splitting sections prematurely. |
| F-03 | MEDIUM | [src/verify/mod.rs#L278-L285](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L278-L285) | Correctness | `audit_links` stores stems in a flat map (`stems.insert(stem, rel)`), causing duplicate note stems across domains to shadow each other and trigger false orphans. |
| F-04 | MEDIUM | [src/search/mod.rs#L472-L478](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/search/mod.rs#L472-L478) | Correctness | `get_keypath` uses `Value::get(&str)`, which fails on sequence indexing (`tags.0`) because `serde_json::Value` only accepts `usize` for arrays. |
| F-05 | MEDIUM | [src/verify/mod.rs#L685-L720](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L685-L720) | Robustness | `run_invariant` does not drain piped stdout/stderr during wait loop; child processes emitting > 64 KB block on pipe buffers and get killed by SIGKILL at timeout. |
| F-06 | LOW | [src/verify/mod.rs#L404](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L404) | Correctness | MOC closure check hardcodes case-sensitive `"INDEX.md"`, causing lowercase `index.md` anchors (as defined in `ROOT_ANCHORS`) to fail indexing closure. |
| F-07 | LOW | [src/verify/mod.rs#L635-L665](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L635-L665) | Security | `run_verification_tests` does not enforce the `repo_test_re` boundary at runtime; unit test assertions (`cargo test`, `pytest`) run if lint is skipped. |
| F-08 | LOW | [src/search/mod.rs#L64-L68](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/search/mod.rs#L64-L68) | Correctness | Hybrid search score scale disparity: empty vector fallback returns raw BM25 (5.0–25.0) whereas fused hits return RRF scores (0.015–0.035). |

## 3. Dimensional Deep Dive

### 3.1 Correctness & Query Engine
- **FTS5 & BM25**: `build_fts_clause` expands queries with suffix trimming and prefix wildcards (`expand_query_term`). `run_bm25_search` correctly applies column weights (`title: 10.0, tags: 5.0, summary: 5.0, body: 1.0`). `ORDER BY score` sorts ascending (correct for negative FTS5 BM25 values), and `SearchHit.score` records `bm25.abs()`.
- **Keypath Resolution**: `get_keypath` correctly resolves `services.<name>` (extracting host, ports, replicas) and `entities.<stem>`. However, `services` discards `parts[3..]`, and neither entity metadata nor note frontmatter resolves numeric array indices (`tags.0`) because `Value::get(&str)` does not index `Value::Array`.
- **Note Mutations**: `write_note` strictly validates YAML frontmatter via `parse_frontmatter` before mutating; missing fields are automatically populated via `auto_heal_frontmatter`. Writes are atomic via PID-tagged temp files and `fs::rename`.

### 3.2 Robustness & Lock Safety
- **SQL Validator Vulnerability**: [src/search/mod.rs#L357-L364](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/search/mod.rs#L357-L364) inspects solely the first token of the SQL query. Mutating queries wrapped in CTEs (`WITH del AS (...) DELETE FROM ...`) pass the check. The validator must verify `stmt.readonly()` on the prepared statement to guarantee read-only execution.
- **Lock Non-Reentrancy**: Public `append_work_log` acquires `VaultLock`; mutation workflows (`write_note`, `replace_section_in_note`, `set_note_property`, `append_section_in_note`) invoke `append_work_log_inner` while already holding `VaultLock`. This cleanly eliminates self-deadlock.
- **Heading Replacement vs Append**: `replace_section_in_note` correctly bails if a heading or note is missing. `append_section_in_note` creates missing headings at the end of the note or synthesizes a seeded note. However, `locate_section` does not track code fences (````...````); bash comments (`# comment`) inside code snippets are parsed as H1 headings, truncating sections prematurely.

### 3.3 Invariant Runner & Security
- **Isolation & Timeouts**: Invariants execute inside a dedicated process group (`process_group(0)`) spawned under `current_dir(vault)`. If the execution duration exceeds `AKATSUKI_INVARIANT_TIMEOUT` (default 10s), `SIGKILL` is sent to `-child.id()`, terminating all subshells cleanly and returning exit code 124.
- **Dry-Run & Pipe Safety**: `dry_run` is safely parsed from both booleans and strings (`arg_bool`). However, in [src/verify/mod.rs#L685-L720](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L685-L720), the wait loop sleeps without draining `child.stdout`/`stderr`. Commands writing > 64 KB will stall on full pipe buffers until timeout.
- **Unit Test Boundary**: `lint_vault` detects `cargo test`, `pytest`, `npm test` inside `bash:verify` blocks, but `run_verification_tests` does not enforce this regex at execution time.

### 3.4 Linting & Link Audit Integrity
- **Graph Closure & Anchors**: `audit_links` strips code fences and inline ticks before link extraction, preventing false positives from code examples. Root anchors (`ROOT_ANCHORS`) and daily notes are exempt from orphan checks.
- **Case Sensitivity & Shadowing**: [src/verify/mod.rs#L404](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L404) checks `sources.contains("INDEX.md")`, which misses lowercase `index.md`. Stems stored in `stems: HashMap<String, String>` overwrite across directories (`20-Projects/api.md` vs `40-Systems/api.md`), causing false orphan alerts.
- **Reconciliation Safety**: `reconcile_vault` auto-quotes unquoted colon scalars while preserving line indentation (`quote_colon_scalars`) and performs writes under `VaultLock` with `write_atomic`.

## 4. Test Suite Evaluation
- **`tests/regression_tests.rs` (178 lines)**: Excellent targeted regression coverage for multibyte snippet char-boundary truncation, unparseable frontmatter rejection, date path-traversal prevention, invariant timeout SIGKILL, MCP dry-run string coercion, JSON-RPC notification silence, and BERT token limit handling.
- **Gaps**: Lacks automated regression tests for mutating CTE SQL rejection, keypath array indexing, code fence comment tolerance in `locate_section`, and stem collision in `audit_links`.

## 5. Prioritized Actionable Remediations
1. **Enforce `stmt.readonly()` in SQL Execution**: Validate `stmt.readonly()` on prepared statements in `execute_sql_query` to block mutating CTEs, and strip leading comments before token inspection ([src/search/mod.rs#L362-L367](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/search/mod.rs#L362-L367)).
2. **Fence-Aware Section Locator**: Update `locate_section` in `src/storage/mod.rs` to track markdown code fences (````...````) so bash comments do not trigger false heading boundaries ([src/storage/mod.rs#L462-L485](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L462-L485)).
3. **Drain Pipes Asynchronously in `run_invariant`**: Read stdout and stderr in background threads or drain pipes iteratively during the wait loop to prevent 64 KB pipe buffer stalls ([src/verify/mod.rs#L685-L729](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L685-L729)).
4. **Support Array Indexing in Keypaths**: Check if keypath segment parses as `usize` in `get_keypath` and index `Value::Array` accordingly ([src/search/mod.rs#L472-L478](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/search/mod.rs#L472-L478)).
5. **Stem Multi-Map in `audit_links`**: Use `HashMap<String, Vec<String>>` for stems to avoid stem shadowing and false orphan reports when note filenames collide across directories ([src/verify/mod.rs#L278-L285](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L278-L285)).
6. **Case-Insensitive Root MOC Resolution**: Use `sources.iter().any(|s| s.eq_ignore_ascii_case("index.md"))` in MOC graph closure check ([src/verify/mod.rs#L404](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L404)).
7. **Runtime Unit Test Gate in `run_verification_tests`**: Reject invariant execution if command matches `repo_test_re` at test execution time ([src/verify/mod.rs#L635-L665](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L635-L665)).
