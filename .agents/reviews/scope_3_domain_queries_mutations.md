---
scope: scope_3_domain_queries_mutations (src/search, src/mutations, src/verify)
score: 6.4
status: FAIL
critical_findings: 3
invariant_breaches: []
---

# Scope 3 — Search, Mutations & Invariant Verification

Stance: Prior(target)=DEFECTIVE. All claims below were reproduced against a throwaway vault in `/tmp` using `target/debug/akatsuki` (vault deleted afterward; no source/test files touched).
Failure vectors evaluated: boundary poison (FTS/SQL/scalars/newlines), concurrency/timeout (child reap, lock windows), resource saturation (limit, SQL cell, pipe cap).

## CRITICAL

### C1. Invariant child inherits stdin — can consume the MCP JSON-RPC stream
[run_invariant](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L758-L765) sets stdout/stderr but never `.stdin(Stdio::null())`.
Counterexample: `cat >/dev/null <&0; echo got-stdin` → exit 124 "Timed out after 10s" (it blocked reading the parent's stdin). Under `akatsuki mcp`, an invariant such as `read x` / `cat` reads bytes out of the MCP request stream, swallowing the next requests and desyncing the host.

### C2. Denylist "security gate" is bypassable; shell exec of vault-sourced text with full environment
[is_dangerous_invariant_command](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L691-L741) is a substring/token denylist; bash is spawned with inherited env ([L758-L765](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L758-L765)).
Executed and PASSED (side effect observed on disk):
- `touch /tmp/s3_bypass1` → file created
- `touch X; find /tmp -maxdepth 1 -name X -delete` → file deleted (`-delete` unguarded)
- `python3 -c 'open("/tmp/s3_b3","w")…'` → file created
- `echo hi | tee /tmp/s3_b4` → file created
Also `mv`, `cp`, `sed -i`, `curl -o`, `eval`, `$(printf 'r\x6d')` are unlisted. `/bin/rm` was blocked only because of the `rm -f` substring pattern ([L736](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L736)). Env leakage: `SECRET_TOKEN=hunter2 … echo $SECRET_TOKEN` runs with the secret visible and its stdout is returned in `TestItem.stdout` ([L603](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L603)) to MCP callers. Trust model: any writer of a note (including `akatsuki_write` via MCP) gets arbitrary code execution on the next `test`. The gate gives a false sense of containment; it should be either removed in favour of an explicit trust statement/opt-in, or replaced with a real sandbox + env scrub (`env_clear` + allowlist).

### C3. Unbounded-time read-only SQL (DoS) in `execute_sql_query`
[execute_sql_query](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/search/mod.rs#L377-L433): readonly guard holds (ATTACH, multi-statement trailer, INSERT-in-CTE, `load_extension`, PRAGMA all rejected; `SELECT 1; DROP TABLE entities` ran only the first statement, `entities` intact), but there is no `progress_handler`/`busy` limit and the row cap ([L400](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/search/mod.rs#L400)) only counts output rows.
Counterexample: `WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c) SELECT count(*) FROM c` ran until killed (5 s timeout, 100% CPU, would never end) — hangs the single-threaded MCP loop. Per-cell size is also uncapped: `SELECT printf('%.*c',100000000,'a')` → 320 MB RSS and ~5 s CPU; `length(printf(200000000))` accepted.

## MODERATE

### M1. Pipe reader stops at 2 MiB → SIGPIPE → false invariant failure
[L777](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L777) `pipe.take(2MiB).read_to_end` then drops the pipe. Counterexample: `head -c 5000000 /dev/zero; echo end` → exit 141, FAILED, even though the assertion would succeed. The cap truncates *and* converts large output into failure (must drain-and-discard the remainder).

### M2. Orphaned grandchildren and silent output loss after normal exit
On `try_wait == Some` ([L803](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L803)) the process group is never killed. `echo done; (sleep 30 &); false` left `sleep 30` alive after `akatsuki test` returned (pgrep confirmed; manually killed). Reader threads stay blocked on the still-open pipe; `recv_timeout(500ms)` ([L823-L828](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L823-L828)) then yields empty stdout/stderr (`unwrap_or_default`) with no marker, i.e. diagnostics silently dropped. `JoinHandle`s are dropped (`_stdout_handle`) → thread leak per invocation. In the MCP daemon each leaked sleeper/thread accumulates.

### M3. No aggregate invariant budget
[run_verification_tests](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L615-L687) runs invariants sequentially, each up to 10 s ([L21](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L21)); N wedged notes ⇒ N×10 s blocked request. No cap on `rows` count, no MCP-level deadline. `AKATSUKI_INVARIANT_TIMEOUT=999999999` is accepted without a ceiling ([L25-L31](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L25-L31)); `Duration::from_secs` of a huge value makes `Instant::now()+timeout` panic on overflow (u64::MAX).

### M4. Unvalidated `limit`: panic and unbounded results
[search_vault L55](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/search/mod.rs#L55): `limit * 2` overflows. Reproduced: `search cache -n 18446744073709551615` (hybrid) → `panicked at src/search/mod.rs:55:30: attempt to multiply with overflow` (release wraps silently). In bm25 mode `limit as i64` ([L200](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/search/mod.rs#L200)) turns `usize::MAX`/2^63 into `-1`/negative, i.e. SQLite `LIMIT -1` = unlimited rows (search accepted both; output unbounded, MCP path takes `arg_usize` at [mcp L147](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L147)). Clamp at the boundary.

### M5. `parse_scalar` silently corrupts string values (DATA_LOSS)
[parse_scalar](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mutations/mod.rs#L477-L500): reproduced via `set -k k -v …`:
- `007` → `k: 7` (leading zeros lost; IDs/zip/phone)
- `1.10` → `k: 1.1` (version string corrupted)
- `+5` → `5`
No way to force string type. Coercion changes user data without a flag.

### M6. Log/audit line injection (forged ledger entries, heading injection)
[append_work_log_inner L146-L153](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mutations/mod.rs#L146-L153) interpolates `summary`/`project`/`device` raw. Reproduced: `log -p x -s $'legit\n## Injected Heading\n- **00:00** [evil]: forged'` wrote a forged bullet and a new `##` heading into `01-Daily/…md`, which also truncates the Work Log section for later `locate_section` appends. Same via [record_audit L417-L425](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mutations/mod.rs#L417-L425): `write $'20-Projects/a\nb.md'` produced an audit line split as `b.md -> exit 0`. Reject control characters at the boundary.

### M7. Section mutations rewrite CRLF notes to LF
[replace_section_in_note](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mutations/mod.rs#L212-L248) and [append_section_in_note](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mutations/mod.rs#L299-L323) use `body.lines()` + `join("\n")`. Reproduced: CRLF note had CRs before; after `append` 0 CR remain (whole-file diff in git for one-line append).

### M8. `reconcile_vault` read-modify-write outside the lock
[L524-L541](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L524-L541) and [L549-L571](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L549-L571): text/MOC is read before `VaultLock::acquire`, and the lock guards only `write_atomic`. A mutation landing between read and write is lost (stale overwrite). Counterexample (by inspection, no race harness): process A `set`s a property on a MOC while B's reconcile has already read it → A's write overwritten. Hold the lock across read→write.

### M9. `quote_colon_scalars` can corrupt YAML
[L483-L500](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L483-L500): (a) lines inside block scalars (`summary: |` + `  a: b`) match the `contains(": ")` test and are rewritten as quoted lines, altering content; (b) only `"` is escaped, `\` is not → `title: C:\path: x` becomes `"C:\path: x"`, an invalid YAML escape that turns a healable note into a parse error. Not executed here (inspection; counterexample inputs given).

## MINOR

- m1. FTS: `build_fts_clause` strips operators via `\w+`, so no MATCH injection reproduced (`NEAR(`, `***`, `"unterminated`, `foo"bar` all safe/empty). However a quoted query with embedded quotes (`"a" b "c"`) becomes a single phrase `a" b "c` → 0 hits instead of 2-term search ([L122-L125](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/search/mod.rs#L122-L125)). Row errors are swallowed by `filter_map(Result::ok)` ([L201-L206](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/search/mod.rs#L201-L206)): a decode failure silently reports "not found", contradicting Retrieval Honesty. `Regex::new` recompiled per call ([L112](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/search/mod.rs#L112)).
- m2. Scores: `abs()*1000 rounded` → a single-doc corpus yields `[Score: 0]` (BM25 IDF≈0); ranking info lost for small vaults; `ORDER BY score` relies on negative-BM25 sign convention (correct) but `abs` drops it from output.
- m3. `execute_sql_query` JSON-parses every TEXT cell ([L417](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/search/mod.rs#L417)): `'true'`→bool, `'null'`→null, `'1e3'`→`1000.0` (reproduced; `'007'` stayed string because JSON rejects leading zeros). Type confusion for consumers; and the 500-row truncation is silent (no marker).
- m4. `ensure_writable_note_path` blocks any component starting `.akatsuki` ([mutations L35](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mutations/mod.rs#L35)), also `.akatsuki-notes`; over-broad but safe.
- m5. `write_note`: after `write_atomic` a failure in `record_audit`/`sync_index` returns `Err` though the note is already committed ([L113-L115](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mutations/mod.rs#L113-L115)) — caller retries with overwrite=false and hits "already exists". Non-atomic multi-file commit.
- m6. Lint boundary-check escape: `cmd.contains("[ -") || contains("hostname")` skips the repo-test rule entirely ([verify L181-L183](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L181-L183)); `cargo test # hostname` passes lint. Gate false positives also observed: `grep -q rm …` and `echo "a>b"` rejected by the invariant denylist.
- m7. `LIKE %filter%` in [run_verification_tests L624-L631](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L624-L631) does not escape `%`/`_`.
- m8. Shell invocation `bash -c "set -eo pipefail;\n…"` + `kill` shell-out ([L789-L796](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L789-L796)) depends on `kill`/`bash` in PATH; PATH-hijackable via env.

## Verified negative results (searched, no counterexample)
Read-only guard: ATTACH/PRAGMA/INSERT-via-CTE/`load_extension`/multi-statement DROP all blocked or inert. FTS MATCH injection: none. Child timeout kill of live process group works (`cat` case → 124 at 10 s).

## Rubric
Score 6.4 / Critical (<7): C1–C3 are exploitable/DoS in a long-lived MCP server; 9 moderate defects. No `invariant_breaches` filed because no declared kernel invariant was violated by a diff (diagnostic scope).
