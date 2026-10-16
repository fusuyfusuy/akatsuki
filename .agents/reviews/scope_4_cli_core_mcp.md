---
scope: scope_4_cli_core_mcp (src/cli/mod.rs, src/mcp/mod.rs)
score: 7.4
status: MODERATE
critical_findings: 0
invariant_breaches: []
---

# Scope 4 — CLI core & MCP stdio gateway (intra-gateway defects)

Method: crafted JSON-RPC piped into `target/debug/akatsuki --vault /tmp/av/v mcp` (1-note temp vault); CLI exit codes probed directly. No source or test files modified. Prior = DEFECTIVE. Gateway-parity seams excluded (see scope_seams.md).

Negative results (vectors tried, no defect): path traversal via `akatsuki_read`/`write_note` (`../`, absolute, `projects/../../x`) rejected; `akatsuki_query` DROP/DELETE rejected, `load_extension` → `not authorized`; `akatsuki_daily` date `../../x` rejected; no stdout pollution (banner on stderr, [L22](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L22)); 50 MB line survived (RSS 154 MB, both pings answered); 50 searches in 0.37 s (1-note vault; per-call reconcile cost not visible at this scale, larger vaults unmeasured); `Cargo.toml` has no `panic=abort`, so [catch_unwind](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L95) is effective (no panic was triggered in probes).

## Findings

### F1 (Moderate) — Batch arrays and non-object messages silently dropped
[mcp/mod.rs#L55-L62](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L55-L62): `req.get("id")` on Array/String/Number yields `None` → treated as notification → skipped.
Counterexample: `[{"jsonrpc":"2.0","id":2,"method":"ping"}]`, `"str"`, `42` → zero output. Spec requires batch handling or `-32600 Invalid Request` with `id:null`. A host awaiting id 2 hangs until its own timeout.

### F2 (Moderate) — Missing `method` → -32601 not -32600; `jsonrpc` version unchecked; id type unchecked
[mcp/mod.rs#L56](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L56), [L128-L132](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L128-L132).
Counterexamples: `{"jsonrpc":"2.0","id":3}` → `-32601 "Method '' not found"`; `{"jsonrpc":"1.0","id":4,"method":"ping"}` → success; `id:{"a":1}` echoed back as valid; `id:null` request answered as a normal request.

### F3 (Moderate) — Unknown tool / missing `params.name` / non-object `params` are not protocol errors
[mcp/mod.rs#L87-L91](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L87-L91): `unwrap_or("")` coerces absent name.
Counterexamples: `params:[]` → `isError:true "Tool '' not implemented"`; `arguments:[]` accepted; unknown tool `nope` → isError result rather than `-32602`. Clients cannot distinguish malformed request from tool-level failure.

### F4 (Moderate) — Invalid UTF-8 on stdin kills the server silently, exit 0
[mcp/mod.rs#L30-L34](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L30-L34): `lines()` returns `Err(InvalidData)` on any non-UTF-8 byte → `break` → `Ok(())`.
Counterexample: `printf '\xff\n'` then valid ping → no `-32700`, ping unanswered, exit code 0 (indistinguishable from clean EOF), nothing on stderr.

### F5 (Moderate) — Argument coercion degrades invalid input silently; `dry_run` fails open
- [L809-L810](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L809-L810) `arg_usize`: `-1`, floats, junk strings → `None` → default limit (verified `limit:-1` returned results); `u64::MAX` accepted uncapped at gateway (verified); `limit:0` returns "No notes found" (same text as a true miss).
- [L145](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L145): `query:123` → `unwrap_or("")` → `isError:false "No notes found matching query."` False negative reported as success.
- [L797-L801](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L797-L801) `arg_bool` unrecognized string → `None`; [L403](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L403) `unwrap_or(false)` → `dry_run:"ture"` (typo) runs live bash assertions. Fail-open on the safety flag; violates DATA_LOSS "no silent fallbacks".

### F6 (Minor) — `akatsuki_query` silently discards trailing statements
Counterexample: `select 1; delete from notes` → `isError:false`, `[{"1":1}]`. Only the first statement runs; the rest vanish without notice. Not exploitable (authorizer verified; `WITH x AS (…) delete` and `load_extension` fail), but misleads caller.

### F7 (Minor) — `akatsuki_test` overloads `isError` with assertion failure
[mcp/mod.rs#L405-L408](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L405-L408): `isError = rep.failed > 0`; completed run with failures is flagged like a crash. CLI `lint` exits 1 on `!passed` ([cli/mod.rs#L541-L543](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/cli/mod.rs#L541-L543)) — consistent with CLI, but MCP clients lose the distinction.

### F8 (Minor) — CLI errors under `--json` are plain text; single exit code
`read nonexist --json` and `query "delete…" --json` print `Error: …` on stderr, rc 1 ([main.rs#L6-L9](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/main.rs#L6-L9) via `Result`). Every failure class (not found, security violation, I/O) → rc 1; `--vault /nonexist read a` reports "Note 'a' not found", masking the missing-vault root cause.

### F9 (Minor) — `blast <unknown>` returns success with empty graph
`blast zzz --json` → rc 0, empty upstream/downstream/boundary_sinks; unknown target indistinguishable from isolated node. MCP path [L238-L257](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L238-L257) identical.

### F10 (Minor) — No bound on request line length
[mcp/mod.rs#L30](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L30): unbounded line buffer; 50 MB line → 154 MB RSS (~3x amplification). Multi-GB line → OOM. Low probability for local stdio.

## Failure-vector ledger (quota ≥3)
| Vector | Result |
|---|---|
| Boundary poison (batch, non-object, bad UTF-8, wrong types) | F1, F3, F4, F5 defects |
| Concurrency/timeout | Single-threaded sequential loop; per-assertion 10 s timeout in `akatsuki_test` but no aggregate cap — N assertions can block the loop N×10 s, no cancellation (static reading, not measured). Panic containment present, untriggered. |
| Resource saturation | F10; `limit` uncapped at gateway (F5) |

## Score
7.4 — no traversal, data-integrity, or stdout-pollution breach found; protocol-conformance gaps (F1–F4) and fail-open `dry_run` coercion (F5) keep it Moderate.
