---
scope: "cross-boundary-seams"
score: 8.2
status: "MODERATE"
contract_divergences: 6
---

# Cross-Boundary Seam & Interface Audit Report

## 1. Executive Summary
An exhaustive interface audit across the 9 subsystems (`cli`, `core`, `mcp`, `mutations`, `search`, `graph`, `index`, `vectors`, `storage`, `verify`) reveals strong path-traversal containment and robust AST search fusion, but exposes six contract discrepancies across serialization error handling, resource MIME negotiation, concurrency locks, subprocess RPC protocols, and schema validation.

---

## 2. Detailed Seam Cross-Checks

### Seam 1: API & RPC Contracts (MCP vs Domain Modules)
- **Tool Parameter & JSON Convention Drift**:
  - `akatsuki_contract` (`mcp/tools.py:88-92`) accepts `"json": bool`, whereas `akatsuki_blast` (`mcp/tools.py:135-140`) and `akatsuki_map` (`mcp/tools.py:166-171`) declare `"format": "text" | "json"`.
  - `akatsuki_test` (`mcp/tools.py:177-193`) omits `json` from its schema, even though underlying `run_verification_tests` (`verify.py:20`) supports structured JSON output (`as_json: bool = False`).
- **Resource MIME Type Contract Violation**:
  - `mcp/server.py:84` declares `mime = "application/json"` for `akatsuki://services` and `akatsuki://projects`.
  - However, when index rows are empty, `mcp/resources.py:58-61` and `mcp/resources.py:72-75` fall back to reading raw Markdown files (`Services-Catalog.md`, `Projects-MOC.md`, `INDEX.md`).
  - *Drift*: JSON-RPC client receives `mimeType: "application/json"` wrapping raw Markdown text.

### Seam 2: CLI vs Core Interface Contracts
- **M2M Error Serialization Discrepancy**:
  - `.agents/memory.md:12` establishes: "All CLI subcommands strictly support `--json` output alongside human terminal formatting."
  - Successful execution correctly returns JSON across commands.
  - Failure paths diverge: `cli_read` (`cli/commands.py:92`), `cli_lint` (`cli/commands.py:234`), and `cli_verify` (`cli/commands.py:390`) output JSON error envelopes to stderr.
  - Conversely, `cli_contract` (`cli/commands.py:151`), `cli_get` (`cli/commands.py:160`), `cli_query` (`cli/commands.py:177`), `cli_blast` (`cli/commands.py:187`), `cli_set` (`cli/commands.py:217`), `cli_append` (`cli/commands.py:257`), `cli_replace` (`cli/commands.py:287`), `cli_write` (`cli/commands.py:445`), and `cli_reconcile` (`cli/commands.py:408`) emit raw plain-text error messages to stderr even when `--json` is enabled.
- **CLI Scaffolding M2M Parity**:
  - `akatsuki init` (`cli/parser.py:201-203`) lacks a `--json` parameter entirely, emitting unstructured text (`cli/parser.py:184`).
- **Unwired Parameter**:
  - `reconcile_vault` (`verify.py:322`) accepts `with_vectors: bool = False`, but `reconcile` CLI parser (`cli/parser.py:346-351`) and MCP schema (`mcp/tools.py:369-381`) fail to expose `--with-vectors`.

### Seam 3: Persistence & DB Seams (FTS5, Vectors, Frontmatter)
- **FTS5 & Vector Queries**:
  - `index.py:38-48` columns match `search.py:200-201` BM25 column weights (`0, 0, 0, 10.0, 5.0, 5.0, 1.0`) and snippet column 6 (`body`).
  - `vectors.py:164-179` (`note_vectors`) matches projection in `vectors.py:419`.
- **Frontmatter Validation Divergence**:
  - `storage.py:426-429` (`validate_note_content`): Enforces `required = ["title", "date", "type", "tags", "summary"]` on ALL notes. Unlisted note types (`note`, `reference`, `database`, `moc`) REQUIRE `tags`.
  - `verify.py:148-162` (`lint_vault`): `required_by_type.get(note_type, ["title", "date", "type", "summary"])` omits `tags` for unlisted types.
  - *Drift*: Notes without `tags` fail write-validation in `storage.py` but pass lint checks in `verify.py`.

### Seam 4: Subprocess Communication Seam (Vectors RPC)
- **Protocol Asymmetry & Injection Hazard**:
  - `encode_texts` (`vectors.py:104-111`): Passes payload safely via `sys.stdin` (`input=json.dumps(texts)`).
  - `encode_query` (`vectors.py:125-139`): Uses inline string formatting (`f"q = {query.strip()!r};"`), creating protocol asymmetry and fragility against query payloads.
- **Unchecked Subprocess Failure & Stdout Corruption**:
  - In `vectors.py:110, 140`, `json.loads(res.stdout)` assumes unpolluted stdout. If external PyTorch/transformers logs warnings or download telemetry to stdout, `json.loads` raises uncaught `json.JSONDecodeError`.
- **Hardcoded Batch Size**:
  - `encode_texts` accepts `batch_size: int = DEFAULT_EMBED_BATCH_SIZE` (`vectors.py:83`), but the subprocess script (`vectors.py:101`) hardcodes `batch_size=32`.

### Seam 5: Documented Invariants & Memory
- **Kernel Lock Bypass in Reconcile**:
  - Invariant: All mutations must serialize under `VaultLock(vault)` and write via atomic rename (`tmp_file` + `os.replace`).
  - `reconcile_vault` (`verify.py:362, 391`) performs in-place writes via `f.write_text(...)` and `parent_moc.write_text(...)` without acquiring `VaultLock(vault)` or using atomic swap files, creating a concurrency seam against concurrent `mutations.py` writes.
- **State Shadowing in `CURRENT_VAULT_OVERRIDE`**:
  - `core.py:76` binds `from akatsuki.storage import CURRENT_VAULT_OVERRIDE`.
  - `storage.py:103-108` prioritizes `core.CURRENT_VAULT_OVERRIDE` over `storage.CURRENT_VAULT_OVERRIDE`. Direct assignment to `storage.CURRENT_VAULT_OVERRIDE` is shadowed if `core.CURRENT_VAULT_OVERRIDE` was previously populated.
- **Scaffolding Omission**:
  - `constants.py:15` specifies `"60-Scripts"` in `DOMAIN_DIRS`, but `cli/parser.py:17-27` (`cli_init`) omits `"60-Scripts"`.

### Seam 6: Path Containment & Security Seams
- **Containment Invariant**:
  - `contained_path` (`storage.py:177-189`) validates `not clean.startswith("/")`, resolves against `vault`, and enforces `vault in target.parents`.
  - Audited call-sites in `mutations.py:95, 172, 222`, `storage.py:324, 327`, and `resolve_note_file` callers in `cli/commands.py` and `mcp/tools.py` maintain strict containment. No directory traversal leaks exist.

---

## 3. Remediation Matrix
| Seam | Source File:Line | Target Seam File:Line | Remediation Action |
|---|---|---|---|
| CLI M2M | `cli/commands.py:151,160,etc` | `.agents/memory.md:12` | Wrap error exits in `json.dumps({"error": ...})` when `args.json` is True |
| MCP MIME | `mcp/server.py:84` | `mcp/resources.py:58,72` | Set `mimeType: text/markdown` when fallback notes are read |
| Storage Lint | `storage.py:426` | `verify.py:161` | Align fallback required fields (`tags` requirement) |
| Vectors RPC | `vectors.py:125-139` | `vectors.py:94-111` | Unify `encode_query` on stdin JSON protocol; filter stdout |
| Concurrency | `verify.py:360-392` | `storage.py:348-378` | Wrap `reconcile_vault` disk writes in `VaultLock(vault)` with atomic temp-files |
| Scaffolding | `cli/parser.py:23` | `constants.py:15` | Add `"60-Scripts"` to `cli_init` subdirs |
