# Project Memory

## Active Epics & Scale
- Scale: Native compiled Rust knowledge secretariat (akatsuki v0.2.0, ~3k LOC Rust across storage, index, search, vectors, graph, mutations, verify, mcp, cli; sub-50ms execution).
- Architecture: Dual-layer Markdown Git source-of-truth with SQLite WAL projection (`.akatsuki/cache.db`), parallel Blake3 Merkle tree change detection via Rayon, pure Rust HuggingFace Candle tensor embeddings (`intfloat/multilingual-e5-small`), and native JSON-RPC 2.0 MCP stdio server.

## KNOWN DEBT (open only — one line per item, delete when done)

## Domain Vocabulary & Gotchas
- Spill Trap: Antigravity dumps tool outputs > 4000 bytes to disk (`output.txt`). MCP `akatsuki_search` caps matches to 5 with dense breadcrumbs (~1.6 KB) to prevent agents wasting turns reading spilled files.
- Domain File Stems: Rust `Path::file_stem()` strips dots (e.g. `yusufakcakaya.com.md` -> `yusufakcakaya`). Notes must use `strip_suffix(".md")` to preserve full domain stems.
- Heading Normalization: Notes frequently use emojis in headings (e.g. `## 📌 Overview`). `extract_section` filters non-alphanumerics before matching query strings.
- Invariant Boundary: Akatsuki never runs repo unit tests; `bash:verify` assertion blocks are strictly for machine infrastructure invariants (ports, containers, daemons, host addresses).
- Parameter Aliasing: `akatsuki_read` accepts `note`, `path`, or `target` interchangeably and normalizes spaces to hyphens for robust stem resolution.
