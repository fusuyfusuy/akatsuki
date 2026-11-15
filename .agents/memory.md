# Project Memory

## Active Epics & Scale
- Scale: Native compiled Rust knowledge secretariat (akatsuki v0.2.4, ~6.7k LOC Rust across storage, index, search, vectors, graph, mutations, verify, mcp, cli; legacy `python/` retired).
- Architecture: Dual-layer Markdown Git source-of-truth with SQLite WAL projection (`.akatsuki/cache.db`, schema version 0.2.4 auto-rebuilt on mismatch), parallel Blake3 content-hash change detection via Rayon, optional pure-Rust Candle embeddings (`intfloat/multilingual-e5-small`) behind `--features vectors` fused with BM25 through RRF, and a native JSON-RPC 2.0 MCP stdio server (JSONL + `Content-Length`) with 20 tools and 6+1 `akatsuki://` resources.

## KNOWN DEBT (open only — one line per item, delete when done)
# Deliberate gaps get ledger lines: - accepted <what> <- <why> -> <trigger>

- e5-small weights (~470 MB) unprovisioned in GitHub Actions <- CI runner cache budget -> when full vector-embedding integration tests must run in CI
- nested frontmatter keypaths lose YAML comments/order <- serde_yaml roundtrip fallback -> when users report comment loss on nested map edits

## Domain Vocabulary & Gotchas
- Spill Trap: Antigravity dumps tool outputs > 4000 bytes to disk (`output.txt`). MCP `akatsuki_search` defaults to 5 matches (limit clamped to [1,100]) with dense breadcrumbs (~1.6 KB) to prevent agents wasting turns reading spilled files.
- Heading Normalization: `storage::locate_section` is the single source of truth for heading lookup; readers (`read --section`) and writers (`replace`/`append`) all route through it. It strips non-alphanumerics, so `--heading Overview` addresses `## 📌 Overview`, and section end = next heading of same-or-higher level.
- Frontmatter Strictness: `parse_frontmatter` returns `Err` for unparseable YAML and for non-mapping roots; fences are column-0 `---` only. Top-level mutations patch the frontmatter text in place (`patch_frontmatter_keys`, comments/order preserved); nested keypaths fall back to a serde roundtrip. Mutations abort instead of rewriting a note from a partial parse; the indexer records such notes in `SyncReport.parse_errors` and `lint` reports them.
- Vault Lock Is Not Re-entrant: `flock` on a second descriptor of `.akatsuki.lock` self-deadlocks in one process. Public `append_work_log` takes the lock; mutation paths call the lock-free `append_work_log_inner` while already holding it.
- Projection Is Derived: `.akatsuki/cache.db` may be deleted at any time; `SCHEMA_VERSION` mismatch drops and rebuilds every table. Every read path uses `index::open_synced_db` so an un-reconciled vault never answers "nothing found".
- Retrieval Honesty: `--mode vector` errors without weights/feature; `hybrid` falls back to BM25 but always prints or returns an explicit `⚠ semantic ranking unavailable…` notice.
- Domain File Stems: Rust `Path::file_stem()` keeps interior dots (`yusufakcakaya.com.md` -> `yusufakcakaya.com`). The legacy Python `Path.stem` stripped the last dot-suffix, which was the bug — Rust is correct; wikilink targets only need their `.md` suffix removed.
- Invariant Boundary: Akatsuki never runs repo unit tests; `bash:verify` assertion blocks are strictly for machine infrastructure invariants (ports, containers, daemons, host addresses). Each assertion runs under a hard timeout (default 10s, `AKATSUKI_INVARIANT_TIMEOUT`; invalid/0 is an error; 300s total budget; env inherited unless `AKATSUKI_INVARIANT_ENV=clean`). Vault-authored shell runs by design (trust model: trusted vaults only).
- Vault Resolution: `storage::resolve_vault_path` returns `Err` for unusable explicit/env paths and when nothing resolves; no cwd fallback; `$HOME` is excluded from the marker walk.
- Scalar Contract: `set` coerces only canonical ints/floats/bools/null (round-trip identical text); everything else stays a string. MCP `akatsuki_set` stores the JSON value as typed; `value` is required.
- Section Writers: `replace`/`append` use `locate_section_exact` and `resolve_note_file_unique` (ambiguous stems are errors); `sync_index` after a write is non-fatal.
- Parameter Aliasing: akatsuki_read, akatsuki_contract, akatsuki_blast, and akatsuki_map accept note, target, or path interchangeably; akatsuki_get accepts key or keypath.
- Code Fence Isolation: storage::locate_section tracks markdown code fences (```/~~~) so that # comments inside bash/python snippets are never mistaken for section headings.
- ARM64 Vector Compilation: ARM64 NEON fp16 assembly in `gemm-common`/`gemm-f16` requires `+fp16` target feature. Configured repo-wide in `.cargo/config.toml` under `[target.aarch64-unknown-linux-gnu] rustflags = ["-C", "target-feature=+fp16"]`. Vector engine (`--features vectors`) and offline execution verified with all unit, cli, and regression suites passing.
