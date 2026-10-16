# Master Architectural Audit Report: Akatsuki v0.2.2 (HEAD fa46b40)

**Generated**: 2026-10-12 | **Mode**: diagnostic only (no source/test edits) | **Swarm**: 4 horizontal + 1 seam auditor (first dispatch hit 429 quota; scopes 2-4 re-dispatched successfully)
Prior (v0.2.0) report superseded. Its S-01 transaction tear is **verified fixed** (`sync_note_vectors_tx` + `file_meta` upserts inside one tx, [index/mod.rs#L442](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L442), [#L786-L795](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L786-L795)).

## 1. Executive Scorecard

| Scope | Subsystem | Score | Status | Critical | Report |
| :--- | :--- | :---: | :---: | :---: | :--- |
| scope_1 | Storage & Markdown | 5.6 | CRITICAL | 4 | [scope_1](scope_1_storage_markdown.md) |
| scope_2 | Index, Vectors, Graph | 6.4 | CRITICAL | 3 | [scope_2](scope_2_index_vectors_graph.md) |
| scope_3 | Search, Mutations, Verify | 6.4 | CRITICAL | 3 | [scope_3](scope_3_domain_queries_mutations.md) |
| scope_4 | CLI & MCP gateway | 7.4 | MODERATE | 0 | [scope_4](scope_4_cli_core_mcp.md) |
| seams | Cross-boundary | 5.8 | CRITICAL | 25 divergences (14 reproduced) | [seams](scope_seams.md) |
| **Overall** | | **6.3** | **CRITICAL** | 10 | |

> Score regressed from 8.9 (v0.2.0 report). Cause is largely deeper adversarial probing (most findings machine-reproduced), not only code change.

## 2. Findings (prioritized)

### P1: Security / data loss / crash
| ID | Finding | Evidence |
| :-- | :-- | :-- |
| 1-C1 | `replace`/`append` fall back to substring heading match; `--heading Log` overwrote `## Work Logs` (reproduced) | [storage#L569](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L569), [mutations#L213](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mutations/mod.rs#L213) |
| 1-C2 | Frontmatter closes on indented `---` inside block scalar; metadata moved into body | [storage#L292-L298](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L292-L298) |
| 1-C3 / seams-20 | `resolve_vault_path` silently falls back to cwd/`$HOME`/real vault when `AKATSUKI_VAULT` invalid; writes land outside intended vault | [storage#L88-L96](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L88-L96), [#L156](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L156) |
| 1-C4 | `VaultLock::acquire` blocks with no deadline | [storage#L30](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L30) |
| 3-C1 | Invariant child inherits stdin; can swallow MCP JSON-RPC stream | [verify#L758-L765](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L758-L765) |
| 3-C2 | Invariant denylist trivially bypassed (`touch`, `find -delete`, `python3 -c`, `tee`); full env leaked to child. Note-write => code exec on `test` | [verify#L691-L741](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/verify/mod.rs#L691-L741) |
| 3-C3 | `query` has no time/size bound (recursive CTE hangs; 1e8-char cell = 320 MB RSS) | [search#L377-L433](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/search/mod.rs#L377-L433) |
| 2-C1 | `graph::target_stem` strips dotted stems (`site.com` -> `site`); false empty blast radius | [graph#L54-L62](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/graph/mod.rs#L54-L62) |
| 2-C2 | Vector embed failure propagates through `open_synced_db`; all reads fail, never converges (code-derived) | [index#L786](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L786) |
| 2-C3 / seams-22,24 | Concurrent reconcile -> `database is locked` (3 of 4 procs); DEFERRED tx, scan outside tx, embed inside write tx | [index#L286-L442](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L286-L442) |
| seams-1 | CI runs Python matrix only; no cargo fmt/clippy/test, no `vectors` leg; `pip install .` fails at root | [ci.yml#L13-L36](file:///home/devhax/projects/fusuyfusuy/akatsuki/.github/workflows/ci.yml#L13-L36) |

### P2: Invariants / robustness (Moderate)
- **Frontmatter lossy roundtrip** (`1.10`->`1.1`, comments/anchors dropped), `dump_frontmatter` `unwrap_or_default` emits empty fm ([storage#L341](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L341)). Scalar coercion `007`->7 ([mutations#L477-L499](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mutations/mod.rs#L477-L499)).
- **Reader/writer heading split & non-CommonMark fences** (1-M1, M2) breach memory.md "single source of truth"/"Code Fence Isolation".
- **Hidden-dir bypass and ambiguous stem resolution** mutate `.trash/ghost.md` or arbitrary duplicate (1-M5, M6).
- **Log/audit newline injection** forges headings/bullets in daily notes (3-M6).
- **Invariant runner**: 2 MiB cap -> SIGPIPE exit 141; process group not killed on normal exit; no aggregate timeout; `limit*2` overflow panic ([search#L55](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/search/mod.rs#L55)).
- **Index**: quadratic FTS delete (3000 notes: 14.9 s cold); stale vectors after `setup-models`; graph duplicate/diamond/dir-name collisions; parse errors reported once.
- **MCP**: batches/non-objects get no reply, bad UTF-8 ends session silently, unknown tool is `isError` not -32602, silent arg coercion (`dry_run:"ture"` runs live assertions) ([mcp#L55-L62](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L55-L62), [#L797-L801](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L797-L801)).
- **Seams**: search "cap 5" is only a default (memory.md claim false); `--json` drops degradation notice; MCP `set` without `value` writes `''`; schema `required` unenforced; mutation error != outcome (retry duplicates); schema rebuild non-atomic vs ADR; budget 4x unit drift.

### P3: Docs / polish
README `-V` flag collides with `--version`; vault ladder and env vars undocumented; query doc drift; three timestamp dialects; `get` JSON shape differs CLI vs MCP; unbounded request line (50 MB -> 154 MB RSS); `blast <unknown>` exits 0.

## 3. Memory/ADR drift (needs ledger update after fixes)
- memory.md "caps matches to 5": false. "Heading Normalization single source": breached. "Frontmatter Strictness": bypassed by block-scalar `---`. "Domain File Stems": breached by `graph::target_stem`.

## 4. Remediation Roadmap (batches)
- **B1 (storage safety)**: 1-C1, 1-C2, 1-C3, 1-C4, M4. Small, one module + mutations caller.
- **B2 (invariant runner)**: 3-C1 (`Stdio::null`), 3-C2 (clear env, drop denylist theater or declare trust model), M1-M3.
- **B3 (index concurrency)**: 2-C2, 2-C3, M1 (index `rel_path`), seams-22/23/24.
- **B4 (graph/search)**: 2-C1, M3-M5, 3-C3 (progress-handler deadline + byte cap), 3-M4.
- **B5 (gateway contracts)**: scope_4 F1-F5, seams 3-6, 9-11, 15, 25; move validation into core.
- **B6 (CI + docs)**: replace ci.yml with Rust matrix; fix README/memory.md.

## 5. Housekeeping
- `Cargo.lock` shows `0.2.1 -> 0.2.2` from `cargo build`; unrelated to audit, commit or revert at your discretion.
- Probe vaults `/tmp/v1`, `/tmp/v2`, `/tmp/av` may remain; safe to delete.
- No test-file changes proposed unless you explicitly request them (policy).
