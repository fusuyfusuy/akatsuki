---
scope: "Scope 4: CLI Interface & MCP Server Gateway"
score: 9.5
status: "EXEMPLARY"
critical_findings: 0
invariant_breaches: []
---

# Scope 4 Audit: CLI Interface & MCP Server Gateway

## 1. Executive Summary
- **Health Score**: 9.5 / 10.0 (EXEMPLARY).
- **Substrate**: Native Clap v4 CLI dispatcher ([`src/cli/mod.rs:20-642`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/cli/mod.rs#L20-L642)) and JSON-RPC 2.0 stdio MCP server ([`src/mcp/mod.rs:21-556`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L21-L556)) exposing 20 architectural knowledge tools.
- **Verdict**: The CLI interface and MCP daemon exhibit exceptional structural rigor. Prior invariant breaches—including exit code 0 masking under `--json` in `lint` and `verify`, uncoerced scalar values in `akatsuki_set`, unvalidated enums in `Map.direction`, and schema parameter omissions—have been comprehensively remediated and verified by regression tests (`tests/regression_tests.rs:1-603`). Panic containment (`catch_unwind`), stdio JSON-RPC notification compliance, and token spill protection (< 2.5 KB responses) are robust. Zero critical findings or invariant breaches remain.

---

## 2. Verification of Prior Remediations
1. **Exit Code 1 Integrity on `--json` Failure**:
   - `std::process::exit(1)` is now positioned after both text and JSON serialization in `Commands::Lint` ([`src/cli/mod.rs:541-543`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/cli/mod.rs#L541-L543)) and `Commands::Verify` ([`src/cli/mod.rs:563-565`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/cli/mod.rs#L563-L565)). CI automation running `--json` correctly fails with exit code 1 on schema/link violations (asserted in [`tests/regression_tests.rs:182-193`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/tests/regression_tests.rs#L182-L193)).
2. **Native Scalar Coercion in `akatsuki_set`**:
   - [`src/mcp/mod.rs:447-455`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L447-L455) parses `Value::String`, `Value::Number`, `Value::Bool`, `Value::Array`, and `Value::Object`, serializing non-string inputs rather than discarding them to empty strings (asserted in [`tests/regression_tests.rs:299-325`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/tests/regression_tests.rs#L299-L325)).
3. **Clap Enum Validation for `Map.direction`**:
   - [`src/cli/mod.rs:86-87`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/cli/mod.rs#L86-L87) enforces `value_parser = ["both", "down", "up"]`, causing invalid directions to cleanly exit 2 via Clap before execution (asserted in [`tests/regression_tests.rs:492-508`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/tests/regression_tests.rs#L492-L508)).
4. **Tool Schema Alignment**:
   - `budget` is formally declared in `akatsuki_read` ([`src/mcp/mod.rs:584`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L584)); `raw` is declared in `akatsuki_write_note` ([`src/mcp/mod.rs:663`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L663)).
5. **Bidirectional Parameter Aliasing**:
   - `akatsuki_contract`, `akatsuki_blast`, and `akatsuki_map` accept `note`, `target`, and `path` interchangeably ([`src/mcp/mod.rs:218-223, 239-244, 260-265`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L218-L223); asserted in [`tests/regression_tests.rs:512-566`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/tests/regression_tests.rs#L512-L566)).

---

## 3. Dimensional Audit Findings

### A. Correctness
- **Clap Parsing & Flag Handling**: Global `--vault` option ([`src/cli/mod.rs:23-24`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/cli/mod.rs#L23-L24)) operates consistently across all 22 subcommands. `allow_hyphen_values = true` on `Write`, `Replace`, and `Append` ([`src/cli/mod.rs:135, 150, 172`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/cli/mod.rs#L135)) allows raw markdown bullet lists (`- item`) without triggering argument syntax errors.
- **Exit Code Conventions**: Syntax errors and invalid enum arguments trigger exit code 2. Verification/lint failures trigger exit code 1. Runtime failures bubble through `anyhow::Result` to exit 1 ([`src/main.rs:6-9`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/main.rs#L6-L9)). All human/json diagnostic outputs route to stdout while error traces route to stderr.
- **JSON-RPC 2.0 Compliance**: Handlers for `initialize`, `tools/list`, `tools/call`, and `ping` conform to MCP protocol version `2024-11-05` ([`src/mcp/mod.rs:65-127`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L65-L127)). Id-less notifications are strictly dropped without generating replies ([`src/mcp/mod.rs:60-62`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L60-L62); [`tests/regression_tests.rs:144-159`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/tests/regression_tests.rs#L144-L159)). Parse errors emit code `-32700` and unknown methods emit `-32601`.
- **Schema Parity**: Exactly 20 tools declared in `get_tool_definitions()` ([`src/mcp/mod.rs:558-786`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L558-L786)) map 1:1 to `dispatch_tool` arms ([`src/mcp/mod.rs:142-555`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L142-L555)).

### B. Robustness
- **Panic Isolation**: Tool execution in `tools/call` is enclosed in `std::panic::catch_unwind(AssertUnwindSafe(...))` ([`src/mcp/mod.rs:95-107`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L95-L107)). Panicking handlers return `{ "content": [...], "isError": true }` with `panic_message` ([`src/mcp/mod.rs:812-820`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L812-L820)) without terminating the daemon.
- **Stdin EOF & Stream Resilience**: Stdin line loop terminates cleanly on EOF ([`src/mcp/mod.rs:30-34`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L30-L34)). Malformed JSON frames emit standard `-32700` responses and continue loop execution without crashing ([`src/mcp/mod.rs:43-52`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L43-L52)).
- **Token Spill Trap Prevention**: Compact search hit formatting (`format_hits_compact`), UTF-8 multibyte truncation ([`tests/regression_tests.rs:31-51`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/tests/regression_tests.rs#L31-51)), and `MCP_DEFAULT_LIMIT = 5` ([`src/constants.rs:36`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/constants.rs#L36)) restrict search responses to ~1.6 KB (under Antigravity 4000-byte spill trap). `akatsuki_read` enforces token budgets via `apply_token_budget` ([`src/storage/mod.rs:629-655`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L629-L655)), preserving frontmatter while truncating body text.

### C. Performance
- **Stdio I/O Buffering**: `stdin.lock().lines()` provides efficient buffered input consumption. Responses are formatted into single strings and flushed immediately via `stdout.flush()?` ([`src/mcp/mod.rs:50, 136`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L50)), preventing pipe deadlocks.
- **Database & Re-Indexing Invariant**: Tool calls requiring database state invoke `open_synced_db(vault)` ([`src/index/mod.rs:110-114`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/index/mod.rs#L110-L114)), which opens SQLite and executes `sync_vault_index` on every invocation ([`src/mcp/mod.rs:287, 319`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L287)). While this guarantees zero stale reads across process boundaries, in a persistent MCP daemon with hundreds of notes, scanning directories and computing BLAKE3 hashes per tool call adds redundant I/O.

### D. Security
- **Path Traversal Protection**: Note resolution and mutations enforce `contained_path` ([`src/storage/mod.rs:192-236`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L192-L236)), strictly blocking directory traversal attacks ([`tests/regression_tests.rs:76-84, 266-272`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/tests/regression_tests.rs#L76-L84)).
- **Read-Only SQL Invariants**: `execute_sql_query` strictly rejects mutating statements and enforces `stmt.readonly()` ([`src/search/mod.rs:377-393`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/search/mod.rs#L377-L393)), blocking mutating CTEs and schema tampering ([`tests/regression_tests.rs:232-261`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/tests/regression_tests.rs#L232-L261)).
- **Invariant Execution Containment**: `akatsuki_test` / `Commands::Test` executes `bash:verify` commands under process groups with strict timeouts (`invariant_timeout()`, default 10s) and asynchronous pipe-draining threads ([`src/verify/mod.rs:687-725`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L687-L725)), preventing hanging processes ([`tests/regression_tests.rs:89-111, 330-353`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/tests/regression_tests.rs#L89-L111)).
- **Information Exposure**: Diagnostic errors return cleanly sanitized messages; sensitive system paths and credentials are not exposed in stdout/stderr error streams.

---

## 4. Minor Residual Polish Items

| Priority | Component | Item & Location | Rationale |
| :--- | :--- | :--- | :--- |
| **Low** | Schema Aliasing | Declare `target` in `akatsuki_read` ([`src/mcp/mod.rs:580`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L580)) & `akatsuki_test` ([`src/mcp/mod.rs:689`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L689)); declare `path` in `akatsuki_map` ([`src/mcp/mod.rs:617`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L617)). | Handler code supports these aliases; documenting them in JSON schema ensures client LLM discovery. |
| **Low** | Daemon DB Cache | Cache SQLite `Connection` and debounce vault hash scans in `run_mcp_server` ([`src/mcp/mod.rs:21`](file:///home/fusuyfusuy/Projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L21)). | Reduces repetitive disk scans during rapid multi-turn MCP tool invocation bursts. |
