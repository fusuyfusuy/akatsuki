---
scope: "cross_boundary_seams"
score: 9.4
status: "MINOR"
contract_divergences: 1
critical_findings: 0
invariant_breaches: []
---

## Executive Summary
The cross-boundary contract interfaces in Akatsuki demonstrate exceptional structural integrity. Parameter coercions are defensively bounded, and process-level safety for the verification engine is airtight. A minor divergence exists between CLI positional arguments and MCP property nomenclature, alongside one potential temporary file leak under strict aborts.

## 1. CLI & MCP ↔ Domain Operations
- **Aliasing & Coercion:** Handled gracefully. `arg_bool` strictly parses `"true"`/`1`/`"yes"` versus booleans [src/mcp/mod.rs#L756](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L756). MCP `akatsuki_read` transparently coalesces `note`, `path`, and `target` [src/mcp/mod.rs#L172](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L172).
- **Error Boundaries:** `run_mcp_server` wraps dispatches in `catch_unwind`, faithfully returning `isError: true` with text logs rather than violating the MCP protocol [src/mcp/mod.rs#L95](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L95).

## 2. Storage ↔ SQLite Projection
- **Read Guarantees:** Total coverage. All read pathways (`akatsuki query`, `search_vault`) traverse through `open_synced_db` [src/index/mod.rs#L67](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L67).
- **Corruption Resilience:** If `cache.db` is destroyed, `open_cache_db` natively rebuilds schemas and triggers an immediate sync, preventing dirty reads.

## 3. Search ↔ Vectors
- **Retrieval Honesty:** Honest degradation. `mode == "vector"` strictly blocks on missing weights or features [src/vectors/mod.rs#L484](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/vectors/mod.rs#L484). Hybrid degradation properly prepends the exact `⚠ semantic ranking unavailable...` signature [src/cli/mod.rs#L254](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/cli/mod.rs#L254).
- **RRF Fusion:** Clean map reduction without panics on empty input matrices [src/search/mod.rs#L218](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/search/mod.rs#L218).

## 4. Mutations ↔ VaultLock
- **Concurrency Check:** `append_work_log` acquires the process-wide `.akatsuki.lock` before calling the unprotected `append_work_log_inner`, fully isolating ledger boundaries [src/mutations/mod.rs#L71](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mutations/mod.rs#L71).
- **Dangling Artifacts:** `write_atomic` cleans up `.tmp` files upon renaming failures. However, if the process SIGKILLs exactly during `fs::write()`, `.tmp` variants may leak [src/storage/mod.rs#L320](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L320).

## 5. Verification ↔ OS Process
- **Dry Runs:** Strictly guarded. `dry_run == true` natively shorts via `continue`, yielding `[DRY-RUN - not executed]` [src/verify/mod.rs#L636](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L636).
- **Orphans:** Avoided. Subprocesses spawn in isolated process groups (`process_group(0)`). Timeout limits trigger a comprehensive group cascade `kill -KILL -<PID>` [src/verify/mod.rs#L677](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L677).

## 6. Contract SKILL Parity
- **Divergence:** 20 tools documented with near-identical parity. Minor contract drift observed: CLI uses positional `<KEYPATH>` for `Get` [src/cli/mod.rs#L107](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/cli/mod.rs#L107) whereas MCP schema enforces `key` [src/mcp/mod.rs#L661](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L661).

## 7. Serialization Boundaries
- **Traversal:** `get_keypath` traverses nested arrays/objects natively (e.g. `entities.<stem>.<field>`) [src/search/mod.rs#L405](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/search/mod.rs#L405).
- **Date Shapes:** `read_daily_note` strictly enforces `YYYY-MM-DD` bounds [src/mutations/mod.rs#L258](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mutations/mod.rs#L258). Modifications serialize using RFC3339 timestamps for `.updated`.
