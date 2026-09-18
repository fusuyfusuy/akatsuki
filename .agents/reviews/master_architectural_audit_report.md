# Master Architectural Audit Report: Akatsuki Knowledge Secretariat

**Audit Date**: 2026-09-18  
**Repository**: `fusuyfusuy/akatsuki`  
**Overall System Health**: **8.12 / 10.0** (`MODERATE`)  
**Audit Protocol**: M2M Boundary Review Protocol v5.0  

---

## 1. Executive Scorecard

| Scope ID | Subsystem / Seam Boundary | Score | Status | Critical / High | Invariant Breaches / Divergences |
| :--- | :--- | :---: | :---: | :---: | :---: |
| **Scope 1** | **[Storage, Markdown & Parsing Engine](file:///home/devhax/projects/fusuyfusuy/akatsuki/.agents/reviews/scope_1_storage_markdown.md)** | 8.2 | Moderate | 2 | 2 breaches |
| **Scope 2** | **[Index, Vectors & Graph Pipeline](file:///home/devhax/projects/fusuyfusuy/akatsuki/.agents/reviews/scope_2_index_vectors_graph.md)** | 7.8 | Moderate | 3 | 2 breaches |
| **Scope 3** | **[Queries, Verification & Mutations](file:///home/devhax/projects/fusuyfusuy/akatsuki/.agents/reviews/scope_3_domain_queries_mutations.md)** | 7.6 | Moderate | 4 | 2 breaches |
| **Scope 4** | **[Adapters: CLI, Core Facade & MCP](file:///home/devhax/projects/fusuyfusuy/akatsuki/.agents/reviews/scope_4_cli_core_mcp.md)** | 8.8 | Minor | 1 | 0 breaches |
| **Scope 5** | **[Cross-Boundary Seams & Interfaces](file:///home/devhax/projects/fusuyfusuy/akatsuki/.agents/reviews/scope_seams.md)** | 8.2 | Moderate | 2 | 6 contract drifts |
| **COMPOSITE** | **Akatsuki Full System Assessment** | **8.12** | **MODERATE** | **12** | **12 total** |

---

## 2. Invariant & Contract Breaches

1. **Arbitrary Unsandboxed Shell Execution**:
   - [`src/akatsuki/verify.py:68-74`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/verify.py#L68-L74) invokes `subprocess.run(cmd, shell=True)` on markdown code blocks (`bash:verify`) without sandbox isolation, permission guards, or working directory confinement.
2. **YAML AST & Indentation Flattening**:
   - [`src/akatsuki/verify.py:345-362`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/verify.py#L345-L362) strips all line indentation during unquoted-colon reconciliation (`k, v = stripped.split(":", 1)`), flattening nested mapping structures into corrupt top-level keys.
3. **Orphan Vector Chunks on Note Truncation**:
   - [`src/akatsuki/vectors.py:356-375`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/vectors.py#L356-L375) keys chunks as `f"{rel}:{i}"` via `INSERT OR REPLACE` but never deletes obsolete higher-indexed chunks when a note shrinks in size, permanently poisoning similarity search.
4. **Subprocess Infinite Hang Vulnerability**:
   - [`src/akatsuki/vectors.py:104-110, 134-139`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/vectors.py#L104-L110) lacks a `timeout` argument during external Python worker execution (`subprocess.run`), risking indefinite process deadlocks during model downloads or GPU initialization.
5. **Daemon Crash via Library `sys.exit`**:
   - [`src/akatsuki/storage.py:173`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L173) executes `sys.exit(1)` inside `get_vault()`. When invoked from MCP tool/resource handlers, missing vault discovery immediately terminates the persistent JSON-RPC daemon process.
6. **Path Traversal Vulnerability in Daily Provisioning**:
   - [`src/akatsuki/storage.py:386`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L386) joins `daily_dir / f"{date_str}.md"` without `contained_path` or format validation, allowing paths like `../../outside` to escape the daily directory and vault root.
7. **Frontmatter Substring Splitting Fragility**:
   - [`src/akatsuki/storage.py:197`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L197) splits markdown frontmatter using raw substring `content.split("---", 2)`. Em-dashes (`---`) inside title or summary fields prematurely split frontmatter, injecting raw YAML into the document body.
8. **Truncated Boundary Sinks at Graph Depth >= 2**:
   - [`src/akatsuki/graph.py:186-200`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/graph.py#L186-L200) populates `all_stems` by iterating only immediate root children (`for c in downstream_tree: all_stems.add(c["stem"])`), silently omitting grandchildren and deeper dependencies from blast radius calculation.
9. **M2M Error Serialization Drift**:
   - While `.agents/memory.md:12` establishes strict M2M JSON formatting, commands (`cli_contract`, `cli_get`, `cli_query`, `cli_blast`, `cli_set`, `cli_append`, `cli_replace`, `cli_write`, `cli_reconcile` in [`src/akatsuki/cli/commands.py`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/cli/commands.py)) dump plain text to stderr on error even when `--json` is specified.
10. **MCP Resource MIME Type Mismatch**:
    - [`src/akatsuki/mcp/server.py:84`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/mcp/server.py#L84) specifies `mimeType: "application/json"` for `akatsuki://services` and `akatsuki://projects`. When index records are empty, [`src/akatsuki/mcp/resources.py:58, 72`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/mcp/resources.py#L58) falls back to returning raw Markdown files wrapped in an `application/json` header.
11. **VaultLock Bypass in Reconcile**:
    - [`src/akatsuki/verify.py:362, 391`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/verify.py#L362) executes in-place `write_text()` without acquiring `VaultLock` or writing through atomic temporary staging, creating concurrency race conditions against `mutations.py`.
12. **Write Amplification Bottleneck**:
    - [`src/akatsuki/mutations.py:270-274`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/mutations.py#L270-L274) invokes `verify_links(vault)` on every single call to `write_note`, forcing a synchronous $O(N)$ full-vault disk scan and re-parsing of every note on each write.

---

## 3. Comprehensive Findings Matrix

| Ref | Severity | Location | Subsystem | Description |
| :--- | :---: | :--- | :--- | :--- |
| **F-01** | **CRITICAL** | [`verify.py:68-74`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/verify.py#L68-L74) | Verification | Arbitrary command injection: executes untrusted vault commands under `shell=True` without sandbox or `cwd` confinement. |
| **F-02** | **CRITICAL** | [`verify.py:345-362`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/verify.py#L345-L362) | Verification | Indentation stripping in `reconcile_vault` flattens nested YAML dictionaries into corrupt top-level keys. |
| **F-03** | **HIGH** | [`vectors.py:356-375`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/vectors.py#L356-L375) | Vectors | Orphan vector chunks persist indefinitely on note edits when text shrinks in length. |
| **F-04** | **HIGH** | [`vectors.py:104-110`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/vectors.py#L104-L110) | Vectors | Subprocess execution lacks `timeout`, risking indefinite hang on model download or PyTorch deadlock. |
| **F-05** | **HIGH** | [`storage.py:173`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L173) | Storage / MCP | `sys.exit(1)` in library function `get_vault()` terminates parent processes (e.g. MCP stdio server). |
| **F-06** | **HIGH** | [`storage.py:386`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L386) | Storage | Path traversal vulnerability in `ensure_daily_note` via unvalidated `date_str`. |
| **F-07** | **HIGH** | [`storage.py:197`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L197) | Storage | `content.split("---", 2)` breaks on em-dashes `---` inside frontmatter fields. |
| **F-08** | **HIGH** | [`mutations.py:270-274`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/mutations.py#L270-L274) | Mutations | $O(N)$ write amplification: full-vault `verify_links` runs synchronously on every `write_note`. |
| **F-09** | **MEDIUM** | [`graph.py:186-200`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/graph.py#L186-L200) | Graph | Truncated boundary sink calculation drops dependencies at depth >= 2. |
| **F-10** | **MEDIUM** | [`verify.py:322-416`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/verify.py#L322-L416) | Verification | Non-atomic, unlocked file writes in `reconcile_vault` risk concurrency corruption. |
| **F-11** | **MEDIUM** | [`index.py:229-234`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/index.py#L229-L234) | Index | Wikilink section anchors (`[[Note#Sec]]`) are not stripped, corrupting relation stems. |
| **F-12** | **MEDIUM** | [`vectors.py:419-451`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/vectors.py#L419-L451) | Vectors | Complete absence of `sqlite-vec` extension; queries perform unindexed full-table pure-Python scans. |
| **F-13** | **MEDIUM** | [`cli/commands.py:151+`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/cli/commands.py#L151) | CLI Seam | Plaintext error strings printed to stderr on failure even when `--json` flag is provided. |
| **F-14** | **MEDIUM** | [`mcp/server.py:84`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/mcp/server.py#L84) | MCP Seam | Resource MIME type declares `application/json` but returns raw Markdown on fallback. |
| **F-15** | **MEDIUM** | [`storage.py:426`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L426) vs [`verify.py:161`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/verify.py#L161) | Storage/Verify | Validation divergence: `storage.py` requires `tags` on all notes; `verify.py` omits `tags` on generic notes. |
| **F-16** | **MEDIUM** | [`storage.py:61-75`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L61-L75) | Storage | Fallback YAML serializer formats lists of dicts as `str(dict)` and fails to escape `\n`. |
| **F-17** | **MEDIUM** | [`markdown.py:23-28`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/markdown.py#L23-L28) | Markdown | Code fence state machine desynchronizes on nested code fences, misclassifying `#` comments as headings. |
| **F-18** | **MEDIUM** | [`search.py:257-288`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/search.py#L257-L288) | Search | Score scale disparity: fallback single-sided hits yield raw BM25 (5-10) vs fused RRF scores (0.01-0.03). |
| **F-19** | **MEDIUM** | [`mutations.py:119-120`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/mutations.py#L119-L120) | Mutations | `append_section_to_note` forcefully prepends `- ` to any content not starting with `#`. |
| **F-20** | **LOW** | [`mcp/tools.py:182, 490`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/mcp/tools.py#L182) | MCP / Tests | Tool schema specifies `note` parameter, but integration test passes `target`, causing silent full test runs. |
| **F-21** | **LOW** | [`cli.py:1-7`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/cli.py#L1) | Packaging | Redundant `cli.py` module shadows package directory `src/akatsuki/cli/`. |
| **F-22** | **LOW** | [`cli/parser.py:23`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/cli/parser.py#L23) | Scaffolding | `akatsuki init` omits `"60-Scripts"` directory specified in `constants.py:DOMAIN_DIRS`. |
| **F-23** | **LOW** | [`tests/test_map_and_search.py`](file:///home/devhax/projects/fusuyfusuy/akatsuki/tests/test_map_and_search.py) | Testing | Zero unit test coverage for `src/akatsuki/vectors.py` chunking, similarity, and sync routines. |

---

## 4. Prioritized Remediation Roadmap

### Phase 1: Security & Crash Immunity (P0 / P1 — High Priority)
1. **Sanitize or Sandbox `bash:verify` Execution**:
   - Require explicit `--allow-exec` flag and execute in a sandboxed subshell with `cwd=vault` ([`verify.py:68-74`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/verify.py#L68-L74)).
2. **Eliminate Library `sys.exit`**:
   - Refactor `get_vault()` in [`storage.py:173`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L173) to raise `VaultNotFoundError(RuntimeError)`. Catch in CLI `main()` (`sys.exit(1)`) and return JSON-RPC error `-32603` in MCP server.
3. **Secure Path Boundaries in Daily Provisioning**:
   - Enforce `contained_path` and regex format check `r"^\d{4}-\d{2}-\d{2}$"` on `date_str` in [`storage.py:386`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L386).
4. **Subprocess Timeout Guard**:
   - Add `timeout=120.0` with `try...except subprocess.TimeoutExpired` in [`vectors.py:104-110, 134-139`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/vectors.py#L104-L110).

### Phase 2: Invariant Correctness & Robustness (P1 / P2 — Medium Priority)
5. **Preserve YAML Hierarchy in Reconcile**:
   - Retain leading whitespace/indentation during auto-quoting in [`verify.py:345-362`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/verify.py#L345-L362).
6. **Prune Stale Vector Chunks**:
   - In [`vectors.py:316-328`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/vectors.py#L316-L328), execute `DELETE FROM note_vectors WHERE rel_path = ?` before inserting updated chunks during `sync_vectors_index`.
7. **Line-Anchored Frontmatter Parsing**:
   - Split frontmatter strictly on line-anchored regex `r"^---\s*$"` in [`storage.py:197`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L197).
8. **Decouple Full Vault Verification from Write Path**:
   - Remove unconditional `verify_links(vault)` from [`mutations.py:270-274`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/mutations.py#L270-L274) or guard behind `verify=False` by default.
9. **Recursively Flatten Graph Boundary Sinks**:
   - Collect all descendant nodes in [`graph.py:186-200`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/graph.py#L186-L200) to resolve boundary sinks at depth >= 2.
10. **Strip Wikilink Section Anchors**:
    - Remove `#anchor` from wikilink targets in [`index.py:229-234`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/index.py#L229-L234) before stem calculation.
11. **Concurrency Protection in Reconcile**:
    - Wrap [`verify.py:360-392`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/verify.py#L360-L392) in `with VaultLock(vault):` and atomic `.tmp.{pid}` staging.

### Phase 3: Seam Parity & Interface Polish (P2 / P3 — Quality of Life)
12. **M2M Error Output Normalization**:
    - Wrap CLI error outputs in `json.dumps({"error": str(e)})` when `args.json` is active across [`cli/commands.py`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/cli/commands.py).
13. **MCP Resource MIME Header Fix**:
    - Set `mimeType: "text/markdown"` when returning fallback notes in [`mcp/resources.py:58-75`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/mcp/resources.py#L58-L75).
14. **Unify Subprocess Input Channels**:
    - Pass query payloads via stdin JSON in [`vectors.py:125-139`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/vectors.py#L125-L139) instead of inline script formatting.
15. **CLI Shadowing & Packaging Clean-up**:
    - Remove redundant [`src/akatsuki/cli.py`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/cli.py) and add `src/akatsuki/cli/__main__.py`.
16. **Add Comprehensive Vector Unit Tests**:
    - Create dedicated test suite for vector chunking, packing, similarity computation, and index synchronization.
