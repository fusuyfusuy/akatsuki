# Project Memory

## Active Epics & Scale
- Scale: Native compiled Rust knowledge secretariat (akatsuki v0.2.0, ~4.5k LOC Rust across storage, index, search, vectors, graph, mutations, verify, mcp, cli).
- Architecture: Dual-layer Markdown Git source-of-truth with SQLite WAL projection (`.akatsuki/cache.db`, schema version 0.2.2 auto-rebuilt on mismatch), parallel Blake3 content-hash change detection via Rayon, optional pure-Rust Candle embeddings (`intfloat/multilingual-e5-small`) behind `--features vectors` fused with BM25 through RRF, and a native JSON-RPC 2.0 MCP stdio server with 20 tools.
- Legacy Python implementation is retained under `python/` as a parity reference only; it is not built, installed, or executed.

## KNOWN DEBT (open only — one line per item, delete when done)
- MCP resource layer (`akatsuki://{note}`) absent <- Python parity gap; no MCP host requests resources yet -> implement `resources/list|read` when a host needs it.
- `python/` legacy tree (3.5k LOC, 30 tracked files) retained <- migration parity reference -> delete once Rust parity is signed off.
- Vector weights (~470 MB e5-small) are a local asset only <- CI has no model provisioning -> provision in CI when vector-path tests must run there.
- `read --toc` / section-miss affordance not implemented <- legacy nicety, low demand -> implement when agents ask for a table of contents.
- `get entities.<domain-stem>.<field>` unreachable for dotted stems (keypath splits on `.`) <- legacy behaviour preserved -> fix with a longest-stem-first keypath resolver when it bites.

## Domain Vocabulary & Gotchas
- Spill Trap: Antigravity dumps tool outputs > 4000 bytes to disk (`output.txt`). MCP `akatsuki_search` caps matches to 5 with dense breadcrumbs (~1.6 KB) to prevent agents wasting turns reading spilled files.
- Heading Normalization: `storage::locate_section` is the single source of truth for heading lookup; readers (`read --section`) and writers (`replace`/`append`) all route through it. It strips non-alphanumerics, so `--heading Overview` addresses `## 📌 Overview`, and section end = next heading of same-or-higher level.
- Frontmatter Strictness: `parse_frontmatter` returns `Err` for unparseable YAML and for non-mapping roots. Mutations abort instead of rewriting a note from a partial parse; the indexer records such notes in `SyncReport.parse_errors` and `lint` reports them.
- Vault Lock Is Not Re-entrant: `flock` on a second descriptor of `.akatsuki.lock` self-deadlocks in one process. Public `append_work_log` takes the lock; mutation paths call the lock-free `append_work_log_inner` while already holding it.
- Projection Is Derived: `.akatsuki/cache.db` may be deleted at any time; `SCHEMA_VERSION` mismatch drops and rebuilds every table. Every read path uses `index::open_synced_db` so an un-reconciled vault never answers "nothing found".
- Retrieval Honesty: `--mode vector` errors without weights/feature; `hybrid` falls back to BM25 but always prints or returns an explicit `⚠ semantic ranking unavailable…` notice.
- Domain File Stems: Rust `Path::file_stem()` keeps interior dots (`yusufakcakaya.com.md` -> `yusufakcakaya.com`). The legacy Python `Path.stem` stripped the last dot-suffix, which was the bug — Rust is correct; wikilink targets only need their `.md` suffix removed.
- Invariant Boundary: Akatsuki never runs repo unit tests; `bash:verify` assertion blocks are strictly for machine infrastructure invariants (ports, containers, daemons, host addresses). Each assertion runs under a hard timeout (default 10s, `AKATSUKI_INVARIANT_TIMEOUT`).
- Parameter Aliasing: akatsuki_read, akatsuki_contract, akatsuki_blast, and akatsuki_map accept note, target, or path interchangeably; akatsuki_get accepts key or keypath.
- Code Fence Isolation: storage::locate_section tracks markdown code fences (```/~~~) so that # comments inside bash/python snippets are never mistaken for section headings.
