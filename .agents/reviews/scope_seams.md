---
scope: "Cross-Boundary Seams & Interfaces"
score: 8.2
status: "MODERATE"
contract_divergences: 0
invariant_breaches: ["Transaction Boundary Tear"]
---

# Cross-Boundary Seam & Interface Audit

## 1. CLI/MCP to Engine Seam (Exemplary)
- **Parameter Aliasing**: Perfect consistency. `akatsuki_read`, `contract`, `blast`, and `map` seamlessly resolve target ambiguities (`note`, `target`, `path`). `akatsuki_get` cleanly aliases `key`/`keypath`. 
- **Boolean Coercion**: `akatsuki_test` and `akatsuki_reconcile` strictly adhere to boolean string coercion via `arg_bool`.
- **JSON Schemas**: CLI `--json` outputs exactly mirror the MCP tool JSON schemas, ensuring zero contract drift.

## 2. Storage & Vault Concurrency Seam (Exemplary)
- **Lock Re-entrancy**: The `append_work_log` (takes lock) vs `append_work_log_inner` (lock-free) split effectively prevents `flock` deadlocks during `record_audit` calls emitted by active mutations holding the lock.
- **Write Atomicity**: `write_atomic` writes to ephemeral `.tmp` files prior to `fs::rename`, completely shielding the SQLite WAL projection from partial writes.

## 3. SQLite Schema & Query Seam (Moderate - Torn Transactions)
- **Schema Parity**: 100% column parity across `search_vault`, `traverse_graph`, and `verify::lint` against `SCHEMA_VERSION 0.2.2`. 
- **Transaction Rollback Safety [Violation]**: In `src/index/mod.rs`, `tx.commit()` flushes `entities` and `notes_fts` updates *before* calling `sync_note_vectors()`. If the vector sync fails, `meta_tx` (which tracks file hashes) never executes. 
  - **Result**: The text projection and graph advance to the new state, but semantic vectors remain stale. This produces torn RRF hybrid search results until the next successful reconcile heals it.

## 4. Documented Invariants & Integration Seam (Exemplary)
- **Pipe Draining**: `run_invariant` prevents buffer deadlocks by draining `stdout` and `stderr` via dedicated `std::thread::spawn` pipelines.
- **Timeout Semantics**: Strongly adheres to `AKATSUKI_INVARIANT_TIMEOUT` with process group SIGKILL on expiry, protecting the agent's context loop from zombie child processes.

## 5. Serialization & Error Propagation Seam (Exemplary)
- **Exit Codes**: `CLI --json` correctly invokes `std::process::exit(1)` upon `lint_vault` or `run_verification_tests` failures, preserving pipeline semantics.
- **UTF-8 Slices**: Token budget truncation, frontmatter scanning, and section extraction operate safely on `&str` splits (`split_inclusive('\n')`), eliminating mid-codepoint panic vectors.
