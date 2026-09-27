---
scope: "Scope 3: Search, Mutations & Invariant Verification"
score: 9.3
status: "MINOR"
critical_findings: 0
invariant_breaches: []
---

# Scope 3 Audit: Search, Mutations & Invariant Verification

## 1. Executive Summary & Health Score
- **Overall Score**: 9.3 / 10 (`MINOR`)
- **Primary Strengths**: All six prior findings (mutating CTEs, code-fence comments in section locator, stem collision in link audits, array indexing in keypaths, invariant pipe buffer starvation, and `index.md` case sensitivity) have been cleanly resolved and verified by regression tests. Locking architecture enforces strict non-reentrant advisory locking (`VaultLock`) with lock-free inner helpers (`append_work_log_inner`). File writes are strictly atomic via temp-rename (`write_atomic`). Path containment (`contained_path`) and daily date regex validation provide airtight directory traversal prevention.
- **Key Remaining Observations**: Memory buffering during invariant stdout/stderr pipe draining is unbounded; `child.try_wait()` error branch leaves subprocess un-reaped; invariant runner boundary check (`repo_test_re`) is only enforced in linter, not at test execution time; regex in `build_fts_clause` is compiled per query.

## 2. Findings Matrix

| Ref | Severity | File:Line | Category | Summary |
|---|---|---|---|---|
| F-01 | LOW | [src/verify/mod.rs:701-729](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L701-L729) | Robustness | `run_invariant` drains pipes into unbounded memory buffers (`Vec::new()`); unhandled `try_wait` error bypasses SIGKILL process group cleanup. |
| F-02 | LOW | [src/verify/mod.rs:655-671](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L655-L671) | Security | `run_verification_tests` does not evaluate `repo_test_re` at runtime; unit test commands execute if `lint` is bypassed. |
| F-03 | LOW | [src/search/mod.rs:64-73](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/search/mod.rs#L64-L73) | Correctness | Hybrid search score discontinuity: fallback without vectors yields raw BM25 (1.0–50.0) whereas fused hits yield RRF scores (0.015–0.035). |
| F-04 | LOW | [src/search/mod.rs:112](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/search/mod.rs#L112) | Performance | `build_fts_clause` compiles `Regex::new(r"\w+")` on every search call instead of reusing a static `LazyLock<Regex>`. |
| F-05 | LOW | [src/search/mod.rs:161](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/search/mod.rs#L161) | Correctness | `run_bm25_search` uses case-sensitive `AND domain = ?2` in FTS5 query, rejecting mismatched casing (e.g. `40-systems`). |
| F-06 | LOW | [src/mutations/mod.rs:217-224](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mutations/mod.rs#L217-L224) | Performance | `append_section_in_note` creates missing note on disk via `create_note` then immediately re-reads, re-parses, and overwrites it. |
| F-07 | LOW | [tests/regression_tests.rs:1-603](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/tests/regression_tests.rs#L1-L603) | Testing | Regression suite lacks positive-path tests for atomic mutation primitives (`write_note` overwrite, `replace_section_in_note`). |

## 3. Verification of Prior Remediations
1. **Mutating CTE & Comment Bypass in `execute_sql_query`**: Resolved in [src/search/mod.rs:378-393](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/search/mod.rs#L378-L393) by stripping comments via `strip_leading_sql_comments` and validating `stmt.readonly()`. Verified by `execute_sql_query_rejects_mutating_ctes`.
2. **Code Fence Comment Splitting in `locate_section`**: Resolved in `src/storage/mod.rs:488-506` by tracking ``` and ~~~ fences. Verified by `section_extraction_ignores_comments_inside_code_fences`.
3. **Stem Collision in `audit_links`**: Resolved in [src/verify/mod.rs:275-288](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L275-L288) by transitioning `stems` to `HashMap<String, Vec<String>>`. Verified by `audit_links_handles_duplicate_stems_across_domains`.
4. **Keypath Array Indexing**: Resolved in [src/search/mod.rs:431-443](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/search/mod.rs#L431-L443) via `traverse_value_keypath` supporting numeric `usize` segments. Verified by `get_keypath_indexes_arrays`.
5. **Invariant Pipe Buffer Exhaustion**: Resolved in [src/verify/mod.rs:701-714](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L701-L714) using dedicated background reader threads. Verified by `run_invariant_drains_large_output_without_timeout`.
6. **Case-Insensitive Root MOC Linkage**: Resolved in [src/verify/mod.rs:409](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L409) using `eq_ignore_ascii_case("index.md")`. Verified by `index_md_lowercase_satisfies_moc_reachability`.

## 4. Dimensional Deep Dive

### 4.1 Correctness: Search, RRF Fusion & Keypath Resolution
- **FTS5 & BM25**: `notes_fts` table matches 7 column weights (`title: 10.0, tags: 5.0, summary: 5.0, body: 1.0, unindexed/domain: 0`). `build_fts_clause` extracts words and provides two-phase matching (AND clause first, OR fallback). Multibyte UTF-8 snippet truncation is completely panic-free via `ellipsize` taking Unicode character counts ([src/search/mod.rs:346-353](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/search/mod.rs#L346-L353)).
- **RRF Fusion**: `fuse_rrf` implements standard reciprocal rank fusion ($k = 60.0$) with 1-based ranking and deduplication via `hit_map` ([src/search/mod.rs:218-247](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/search/mod.rs#L218-L247)).
- **Keypath Navigation**: Dotted keypaths support `services.<svc>.<prop>`, `entities.<stem>.<prop>`, and frontmatter resolution, handling nested JSON maps and arrays cleanly ([src/search/mod.rs:431-548](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/search/mod.rs#L431-L548)).

### 4.2 Robustness: Lock Safety & Atomic Mutation Primitives
- **Lock Management**: `VaultLock` acquires an exclusive `flock` on `.akatsuki.lock` at the entry point of all public mutation APIs ([src/mutations/mod.rs:31, 77, 152, 190, 215](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mutations/mod.rs#L31)). Helpers (`append_work_log_inner`, `create_note`, `record_audit`) are explicitly lock-free, preventing self-deadlock.
- **Atomic I/O & Non-Existent Notes**: All note writes use PID+nanosecond temporary files and atomic `rename` via `write_atomic` ([src/mutations/mod.rs:64, 142, 182, 202, 251](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mutations/mod.rs#L64)). `replace_section_in_note` cleanly fails when target note or heading is absent; `append_section_in_note` auto-creates missing headings or seeds missing notes.
- **Audit & Provenance**: Every mutation automatically updates `updated` (RFC3339) and `updated_by` (`machine_id()`) in frontmatter and appends an audit entry to the daily ledger ([src/mutations/mod.rs:332-351](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mutations/mod.rs#L332-L351)).

### 4.3 Invariant Runner: Execution, Timeouts & Pipe Draining
- **Process Isolation & Ceilings**: Invariants execute in a detached process group (`process_group(0)`). On expiration of `AKATSUKI_INVARIANT_TIMEOUT` (default 10s), `Command::new("kill").arg("-KILL").arg(format!("-{}", child.id()))` kills the entire process group, returning exit code 124 ([src/verify/mod.rs:683-759](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L683-L759)).
- **Concurrent Pipe Draining**: Dedicated threads drain child stdout/stderr pipes, preventing 64 KB pipe buffer stalls ([src/verify/mod.rs:701-714](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L701-L714)).

### 4.4 Security: Injection Defenses & Sandbox Confinement
- **SQL Execution**: `execute_sql_query` strips comments, enforces keyword whitelisting (`SELECT|WITH|EXPLAIN`), and mandates `stmt.readonly()` ([src/search/mod.rs:377-393](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/search/mod.rs#L377-L393)). Parameterized Rusqlite queries are strictly used for BM25 and entity queries.
- **Path Traversal**: `contained_path` strictly rejects paths resolving outside the vault boundary or underflowing relative paths. `read_daily_note` enforces exact byte-level `YYYY-MM-DD` validation ([src/mutations/mod.rs:262-274](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mutations/mod.rs#L262-L274)).
- **Frontmatter Injection**: `set_note_property` strictly parses and serializes via `serde_yaml` AST serialization, preventing string interpolation injection ([src/mutations/mod.rs:189-207](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mutations/mod.rs#L189-L207)).

## 5. Test Suite Evaluation
- **Current Coverage**: `tests/regression_tests.rs` contains 21 regression tests covering multibyte truncation, unparseable frontmatter rejection, date traversal rejection, invariant timeout, large pipe draining (>128 KB), dry-run string parsing, mutating CTE rejection, keypath array indexing, and MOC reachability. All 21 tests execute and pass in ~1.0s.
- **Gaps**: Positive-path tests for `replace_section_in_note` and `append_section_in_note` are absent from `regression_tests.rs`.

## 6. Actionable Recommendations
1. **Bounded Buffer Draining**: In [src/verify/mod.rs:701-714](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L701-L714), use `pipe.take(MAX_CAPTURE_BYTES).read_to_end(&mut buf)` (e.g. 2 MB) to prevent runaway invariant memory consumption.
2. **Reap Child on Wait Failure**: In [src/verify/mod.rs:723-729](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L723-L729), send `SIGKILL` to the process group and call `child.wait()` if `try_wait` returns `Err(_)`.
3. **Static Regex Compilation**: Use `std::sync::LazyLock<Regex>` for `r"\w+"` in `build_fts_clause` ([src/search/mod.rs:112](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/search/mod.rs#L112)).
4. **Runtime Invariant Boundary Guard**: Check `repo_test_re` in `run_verification_tests` ([src/verify/mod.rs:655](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L655)) before spawning bash commands.
