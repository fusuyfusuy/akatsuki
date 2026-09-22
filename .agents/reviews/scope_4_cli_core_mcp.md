---
scope: "cli_core_mcp"
score: 8.6
status: "MINOR"
critical_findings: 0
invariant_breaches:
  - "CLI Commands::Lint and Commands::Verify return exit code 0 when --json is specified on failure (violates SKILL.md exit code contract)"
---

# Scope Audit: CLI Interface & JSON-RPC MCP Daemon (Akatsuki v0.2.0)

## 1. Executive Summary
- **Health Score**: 8.6 / 10.0 (MINOR).
- **Core Substrate**: Native Clap v4 CLI dispatcher and JSON-RPC 2.0 stdio MCP daemon exposing 20 architectural knowledge tools.
- **Verdict**: The CLI and MCP adapters demonstrate solid engineering: clean panic isolation (`catch_unwind`), strict notification ignoring (no responses to id-less frames), resilient multi-tier vault discovery, and effective 1.6 KB compact search formatting preventing agent spill traps. However, an exit code contract breach in `lint --json` and `verify --json` masks failures in automation, MCP parameter type coercion fails on non-string scalars in `akatsuki_set`, and several tool schemas omit documented arguments (`budget`, `raw`).

---

## 2. Invariant Breaches

1. **CLI `lint --json` and `verify --json` Return Exit Code 0 on Failure**:
   - **Contract**: [`SKILL.md#L84`](file:///home/devhax/projects/fusuyfusuy/akatsuki/SKILL.md#L84) defines `akatsuki lint ∧ akatsuki verify == exit 0` as the integrity gate, and [`SKILL.md#L205-L206`](file:///home/devhax/projects/fusuyfusuy/akatsuki/SKILL.md#L205-L206) specifies exit code `1` for schema lint errors and broken links.
   - **Breach**: In [`src/cli/mod.rs#L531-L541`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/cli/mod.rs#L531-L541) (`Commands::Lint`) and [`src/cli/mod.rs#L545-L561`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/cli/mod.rs#L545-L561) (`Commands::Verify`), `std::process::exit(1)` is sequestered inside the `else` (non-JSON) branch. When `--json` is supplied, both commands serialize the report and return `Ok(())`, exiting `0` even if `rep.passed == false`. In contrast, [`Commands::Test` in src/cli/mod.rs#L599-L601](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/cli/mod.rs#L599-L601) correctly positions `if rep.failed > 0 { std::process::exit(1); }` outside the formatting branch. Automated CI/CD gates running `akatsuki lint --json` falsely pass green on invalid vaults.

---

## 3. Dimensional Findings

### A. Correctness
- **MCP Tool Schema Omissions** ([`src/mcp/mod.rs#L544-L555`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L544-L555), [`L621-L632`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L621-L632)):
  In `get_tool_definitions()`, `akatsuki_read` declares `note`, `path`, and `section`, but omits `budget`, despite `dispatch_tool` extracting it ([`src/mcp/mod.rs#L189`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L189)) and [`SKILL.md#L157`](file:///home/devhax/projects/fusuyfusuy/akatsuki/SKILL.md#L157) advertising it. Similarly, `akatsuki_write_note` declares `path`, `content`, and `overwrite`, but omits `raw`, despite `dispatch_tool` extracting `raw` ([`src/mcp/mod.rs#L347`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L347)) and [`SKILL.md#L164`](file:///home/devhax/projects/fusuyfusuy/akatsuki/SKILL.md#L164) documenting it.
- **Unvalidated CLI Enum on `Map.direction`** ([`src/cli/mod.rs#L86-L88`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/cli/mod.rs#L86-L88)):
  `Search.mode` validates allowed modes via Clap's `value_parser = ["hybrid", "bm25", "vector"]` (exiting `2` on invalid input). `Map.direction` has no `value_parser`; invalid arguments (e.g. `--direction sideways`) silently fall back to `"both"` in [`src/graph/mod.rs#L449-L453`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/graph/mod.rs#L449-L453) instead of raising exit code `2`.
- **JSON-RPC 2.0 Protocol Compliance** ([`src/mcp/mod.rs#L41-L137`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L41-L137)):
  Exemplary method routing for `initialize`, `tools/list`, `tools/call`, and `ping`. Parse errors properly emit error `-32700` with `id: null`. Tool registration contains exactly 20 tools with 1:1 match parity in `dispatch_tool`.

### B. Robustness
- **Type Coercion Dropout in `akatsuki_set`** ([`src/mcp/mod.rs#L425`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L425)):
  `let value = args.get("value").and_then(|v| v.as_str()).unwrap_or("");` assumes string input. If a host sends native JSON booleans, numbers, or arrays (e.g. `{"value": 42}` or `{"value": true}`), `v.as_str()` returns `None`, silently wiping the target property to `""`. MCP schema ([`src/mcp/mod.rs#L690`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L690)) explicitly documents accepting numbers, booleans, and arrays; coercion must stringify non-string values before passing to `set_note_property`.
- **Panic Isolation & Notification Safety** ([`src/mcp/mod.rs#L59-L62`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L59-L62), [`L94-L107`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L94-L107)):
  `catch_unwind` wraps `dispatch_tool`, safely returning `{ content: [...], isError: true }` on panics and preserving daemon life. Notifications without `id` are silently skipped (asserted by regression suite [`tests/regression_tests.rs#L144-L159`](file:///home/devhax/projects/fusuyfusuy/akatsuki/tests/regression_tests.rs#L144-L159)). Loose booleans (`"true"`, `"1"`, `"yes"`) in `arg_bool` prevent assertion leakage in dry-run mode ([`src/mcp/mod.rs#L756-L766`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L756-L766)).

### C. Usability & Spill Prevention
- **Spill Trap Prevention (< 4000 Bytes)** ([`src/constants.rs#L36-L37`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/constants.rs#L36-L37), [`src/mcp/mod.rs#L147`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L147)):
  MCP default limit of 5 combined with `format_hits_compact` and 160-char ellipsized snippets bounds typical search responses to ~1.6 KB, comfortably below the Antigravity 4000-byte tool spill trap. CLI defaults to 10 hits (`CLI_DEFAULT_LIMIT`).
- **Terminal vs JSON Duality** ([`src/cli/mod.rs#L249-L635`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/cli/mod.rs#L249-L635)):
  All 20 functional CLI commands provide structured `--json` outputs. Human terminal modes output rich domain representations (compact hits, token-dense YAML contracts, emoji blast trees, ASCII graph maps, NDJSON queries).

### D. Boundary Cleanliness
- **Asymmetric Parameter Aliasing in Graph/Contract Tools** ([`src/mcp/mod.rs#L218`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L218), [`L234`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L234), [`L250`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L250)):
  While `akatsuki_read` aliases `note`, `path`, and `target` ([`src/mcp/mod.rs#L174-L177`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L174-L177)) and `akatsuki_test` aliases `note` and `target`, `akatsuki_contract` strictly requires `note` (rejects `target`), and `akatsuki_blast` / `akatsuki_map` strictly require `target` (rejects `note`). Unifying `target` and `note` across contract and graph tools prevents common LLM invocation failures.
- **Vault Discovery & Path Resolution** ([`src/storage/mod.rs#L71-L148`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L71-L148), [`src/cli/mod.rs#L23-L24`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/cli/mod.rs#L23-L24)):
  Global `--vault` attribute operates cleanly before and after subcommands. Fallback ladder resolves CLI arg -> `$AKATSUKI_VAULT` (with `shellexpand`) -> `~/.config/knowledge-base/env` -> ancestor search -> known defaults -> `.`. `contained_path` ([`src/storage/mod.rs#L192-L236`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L192-L236)) enforces containment against traversal attacks.

---

## 4. Prioritized Actionable Remediations

| Priority | Component | Remediation |
| :--- | :--- | :--- |
| **P0 (Critical)** | `Commands::Lint` & `Commands::Verify` | Move `if !rep.passed { std::process::exit(1); }` outside the `if json { ... } else { ... }` block in [`src/cli/mod.rs#L540`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/cli/mod.rs#L540) and [`L560`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/cli/mod.rs#L560) to guarantee exit code `1` on failure in `--json` mode. |
| **P1 (High)** | `akatsuki_set` MCP Dispatch | Support non-string JSON values in [`src/mcp/mod.rs#L425`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L425): coerce `Value::Number`, `Value::Bool`, and structured JSON into string representations before passing to `set_note_property`. |
| **P1 (High)** | MCP Tool Definitions | Add missing `budget` property to `akatsuki_read` ([`src/mcp/mod.rs#L548`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L548)) and missing `raw` boolean property to `akatsuki_write_note` ([`src/mcp/mod.rs#L625`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L625)). |
| **P2 (Medium)** | `Commands::Map` Clap Parser | Add `#[arg(long, default_value = "both", value_parser = ["both", "down", "up"])]` to `direction` in [`src/cli/mod.rs#L86-L88`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/cli/mod.rs#L86-L88) to enforce valid arguments with exit code `2`. |
| **P2 (Medium)** | Parameter Aliasing | In [`src/mcp/mod.rs#L218,L234,L250`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L218), accept `note` or `target` interchangeably across `akatsuki_contract`, `akatsuki_blast`, and `akatsuki_map`. |
