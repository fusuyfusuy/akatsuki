---
scope: "cli-core-mcp"
score: 8.8
status: "MINOR"
critical_findings: 0
invariant_breaches: []
---

# Scope 4 Audit: Adapters Auditor (CLI, Core Facade & MCP)

## Executive Summary
Comprehensive audit of Akatsuki's external interfaces (`core.py`, `akatsuki.cli`, and `akatsuki.mcp`). The adapter layer provides robust isolation, zero stray `print()` pollution during MCP operation, strict vault containment against traversal attacks, read-only SQL enforcement, and sub-90ms cold start latency. 45/45 regression tests pass. Minor issues identified include core storage layer invoking `sys.exit(1)`, redundant/shadowing `cli.py` module, inconsistent M2M JSON error formatting on CLI stderr, and argument aliasing discrepancy in `akatsuki_test`.

---

## 1. Correctness & Protocol Compliance
- **Core Facade Parity (`core.py:108-199`)**: Exemplary. Complete backward-compatible facade re-exporting 90 public symbols across storage, index, vectors, search, graph, verify, mutations, CLI, and MCP. Zero missing or broken export bindings.
- **MCP JSON-RPC 2.0 Stdio Compliance (`mcp/server.py:11-136, 138-192`)**: Fully implements request/response framing, notifications (silent ping/initialized), batch request execution, and standard error codes (-32700, -32600, -32601, -32602, -32603).
- **Resource Handlers (`mcp/resources.py:9-46, 49-109`)**: Correctly advertises 6 static resources plus dynamic note URI templates (`akatsuki://{note}`). Resources map cleanly to live FTS SQLite index or vault markdown files with explicit MIME type typing (`application/json` vs `text/markdown`).
- **Tool Dispatch Parity (`mcp/tools.py:22-382, 385-619`)**: 20/20 defined tools in `MCP_TOOLS` have direct execution branches in `handle_mcp_call` with zero orphans.

---

## 2. Robustness & Fault Tolerance
- **Core Storage Invoking `sys.exit(1)` (`storage.py:166-175`)**: `get_vault()` writes error message to stderr and abruptly invokes `sys.exit(1)` when the vault directory cannot be discovered. Because MCP tool calls (`mcp/tools.py:386`) and resource reads (`mcp/resources.py:50`) invoke `get_vault()`, an uninitialized vault crashes the entire MCP server process rather than returning a JSON-RPC error response (-32603) or `isError: True`.
- **MCP Single-Request Unhandled Exception Vulnerability (`mcp/server.py:188-192`)**: While batch requests wrap each item in a `try...except` block mapping unexpected errors to code `-32603`, single requests call `dispatch_single_request(req)` without an outer `try...except` in the loop. Any exception in dispatcher prelude crashes the stdio server.
- **Stdout Purity in MCP Mode**: Verified 100% clean. Zero unbuffered `print()` or logging statements exist across core, storage, index, vectors, graph, or MCP modules. All console outputs are strictly isolated inside `cli/commands.py` and `cli/parser.py`.
- **Inconsistent CLI M2M Error Formatting (`cli/commands.py:151, 160, 177, 187, 217, 236, 257, 287, 408, 445`)**: Commands such as `read`, `append`, `replace`, and `verify` emit structured JSON on failure when `--json` is supplied (`{"error": "..."}` or `{"passed": false}`). However, `contract`, `get`, `query`, `blast`, `set`, `lint`, `reconcile`, and `write` dump raw plain text to stderr on error even when `--json` was passed, breaking machine-to-machine parsing.

---

## 3. Performance & Resource Footprint
- **Startup Latency**: Cold import of `akatsuki.core` completes in ~43ms. CLI execution (`akatsuki --help`) completes in ~89ms. Heavy dependencies (`sentence-transformers`, `torch`) are lazy-loaded on demand only when vector operations are triggered.
- **Tool Definition Overhead**: All 20 MCP tools serialize to ~8.8 KB of JSON in memory, posing minimal heap footprint and instant schema negotiation during client handshake.

---

## 4. Security & Containment
- **Path Traversal Containment (`storage.py:177-189, 313-347`)**: `contained_path()` strictly validates relative paths against `vault.resolve()`, blocking directory traversal (`../`, absolute paths `/`, `~`). Applied consistently in `resolve_note_file()`, `write_note()`, `append_section_to_note()`, and `replace_section_in_note()`.
- **SQL Injection Prevention (`index.py:306-339`)**: `execute_sql_query()` validates query tokens against write keywords (INSERT, UPDATE, DELETE, DROP, ALTER, CREATE, ATTACH, DETACH), restricts verbs to SELECT/WITH/EXPLAIN, and enforces SQLite URI read-only connection mode (`mode=ro`).
- **Concurrency Protection (`storage.py:349-380`)**: All vault mutations acquire process-safe kernel advisory lock (`VaultLock` via `fcntl.flock`).

---

## Detailed Findings

### [MEDIUM] Storage Layer `get_vault()` Hard Exits Process
- **Location**: `src/akatsuki/storage.py:173`
- **Impact**: Server termination in daemon/MCP mode if vault path is absent.
- **Remediation**: Raise `VaultNotFoundError(RuntimeError)` in `storage.py`; catch in CLI `main()` (`sys.exit(1)`) and MCP `dispatch_single_request()` (return JSON-RPC error -32603).

### [LOW] Redundant `cli.py` Module Shadows `cli/` Package
- **Location**: `src/akatsuki/cli.py:1-7`, `src/akatsuki/cli/__init__.py:1-96`
- **Impact**: Having both `akatsuki/cli.py` and package directory `akatsuki/cli/` creates import shadowing ambiguity and packaging traps. `cli.py` imports `main` from `core`, while `core` imports `main` from `cli`. `python -m akatsuki.cli` fails because `cli/__main__.py` is absent.
- **Remediation**: Remove `src/akatsuki/cli.py` or move entrypoint script logic into `src/akatsuki/cli/__main__.py`.

### [LOW] MCP Tool Argument Mismatch in `akatsuki_test`
- **Location**: `src/akatsuki/mcp/tools.py:182-185, 490` vs `tests/test_e2e.py:397`
- **Impact**: `akatsuki_test` input schema defines `note` parameter, but integration test passes `target`. Unmatched `target` causes handler to fall back to `note=None`, silently executing all vault tests instead of the specified note.
- **Remediation**: Accept `target = args.get("note") or args.get("target")` in `handle_mcp_call`.

### [LOW] Unhandled Exception In MCP Single-Request Loop
- **Location**: `src/akatsuki/mcp/server.py:188-192`
- **Impact**: Single malformed request throwing unanticipated exception crashes stdio loop.
- **Remediation**: Wrap line 188 in `try...except Exception as e:` and output JSON-RPC error code `-32603`.

### [LOW] Inconsistent `--json` Error Output on CLI Failures
- **Location**: `src/akatsuki/cli/commands.py:151, 160, 177, 187, 217, 236, 445`
- **Impact**: CLI stderr outputs plain text strings on error even when `--json` flag is supplied.
- **Remediation**: Standardize error handler in `commands.py` to emit `json.dumps({"status": "error", "message": str(e)})` when `args.json` is True.

### [INFORMATIONAL] CLI `query` Flag Redundancy
- **Location**: `src/akatsuki/cli/parser.py:249`, `src/akatsuki/cli/commands.py:173-180`
- **Impact**: `akatsuki query` has a `--json` argument but outputs JSON unconditionally regardless of flag value.
- **Remediation**: Add tabular text formatter when `--json` is false, or mark `--json` as default in documentation.

---

## Prioritized Actionable Remediations
1. **P1 (Core/MCP Resilience)**: Refactor `get_vault()` in `storage.py` to raise `VaultNotFoundError` instead of calling `sys.exit(1)`; handle cleanly in `mcp/server.py` and `cli/__init__.py`.
2. **P1 (MCP Stdio Stability)**: Add `try...except` around single-request dispatch in `mcp/server.py:188` to return JSON-RPC `-32603` and prevent server termination.
3. **P2 (CLI Packaging Hygiene)**: Delete redundant `src/akatsuki/cli.py` and add `src/akatsuki/cli/__main__.py` pointing to `akatsuki.cli:main`.
4. **P2 (Agent Ergonomics)**: Add parameter alias `target` -> `note` in `mcp/tools.py:490` for `akatsuki_test`.
5. **P3 (M2M Formatting Consistency)**: Standardize error output in `cli/commands.py` so all subcommands output JSON to stderr when `--json` is enabled.
