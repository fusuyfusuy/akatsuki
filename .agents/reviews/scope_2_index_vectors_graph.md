---
scope: "Scope 2 — Indexing, Vectors & Knowledge Graph (src/index, src/vectors, src/graph)"
score: 6.4
status: FAIL_CRITICAL
critical_findings: 3
invariant_breaches: []
---

# Scope 2 Audit — Index / Vectors / Graph (akatsuki v0.2.2, HEAD fa46b40)

Method: static read of all three modules + throwaway CLI probes against `/tmp/v1`, `/tmp/v2` using `target/debug/akatsuki` (built **without** `vectors` feature). No source/test edits. Prior = DEFECTIVE.

## 0. Prior-audit tx tear — status: FIXED (structurally)
- `sync_note_vectors_tx(&tx, …)` now runs inside the single `con.transaction()` ([index/mod.rs#L442](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L442), [#L786](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L786)); `file_meta` upserts ([#L788-L793](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L788-L793)) precede the single `tx.commit()` ([#L795](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L795)). Any `?` drops `tx` → rollback. FTS/entities/vectors/file_meta are atomic.
- Residual cost of the fix: see C2 (vector failure now aborts the *entire* lexical sync) and C3 (write lock held across inference).

## Critical (< 7)

### C1 — Dotted-stem notes unreachable in blast/map/contract-by-target (data-correctness)
[graph/mod.rs#L54-L62](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/graph/mod.rs#L54-L62) `target_stem` strips `.md` then calls `Path::file_stem`, which strips the *next* extension. Indexer stems keep dots ([index/mod.rs#L483-L486](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L483-L486)), contradicting memory.md "Domain File Stems".
Counterexample (machine-verified): note `infra/site.com.md`, `proj/E.md` links `[[site.com]]`. `akatsuki blast site.com` → header `` `site` ``, upstream/downstream/sinks all **empty** (expected E.md as dependent). Blast radius silently reports "none" for every domain-named note — false-safe output on the exact tool used as BLAST_GATE.
Same root defect class: `path_stem` ([#L65-L71](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/graph/mod.rs#L65-L71)) is fine for `.md` paths, but `target_stem` is not.

### C2 — Vector failure poisons all read paths (availability coupling)
[index/mod.rs#L786](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L786) `?` on `sync_note_vectors_tx`; inside, `CandleEmbedder::load()?`, `encode_passage(..)?`, dim-mismatch `bail!` ([vectors/mod.rs#L507-L525](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/vectors/mod.rs#L507-L525)) all propagate. Every read command goes through `open_synced_db` ([index/mod.rs#L126-L130](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L126-L130)), so one corrupt `config.json`/truncated safetensors, tokenizer error, or OOM-returned `Err` makes `search`, `blast`, `contract`, `map`, `services`, `get` all fail while FTS-only answers are possible. Retry never converges (file_meta rolled back → same work each call). Not machine-run (feature off); derived from code path.

### C3 — Concurrent reconcile: immediate `database is locked`; write lock spans CPU inference
Probe: 4 parallel `reconcile` on `/tmp/v2` (3000 notes, 4 files touched): **3 of 4 exited `Error: database is locked` in <100 ms**; `busy_timeout(5s)` ([#L110](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L110)) does not protect the snapshot→write upgrade: hashes are read ([#L286-L295](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L286-L295)) *outside* the tx, the tx is `DEFERRED` ([#L442](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L442)), diff computed on a stale snapshot. Also `journal_mode=WAL`/`DROP TABLE` schema rebuild ([#L111](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L111), [#L146-L165](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L146-L165)) run un-transacted and un-flocked at open. Read commands (map/blast/search/MCP server) all call this, so two agents hitting one vault race — the vault `flock` covers only mutations.
With the vectors feature the first DELETE takes the write lock ([#L445](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L445)/[#L553](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L553)) and it is held while every chunk is embedded on CPU ([vectors/mod.rs#L510-L548](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/vectors/mod.rs#L510-L548)): any other reader-syncer exceeds the 5 s timeout for any non-trivial vault.
Fix direction: `BEGIN IMMEDIATE`, re-read hashes inside tx; embed outside tx, write inside.

## Moderate (7–8.4)

### M1 — Quadratic FTS maintenance
`DELETE FROM notes_fts WHERE rel_path=?` targets an UNINDEXED column → full FTS scan per note ([index/mod.rs#L450](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L450), [#L553](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L553)); executed even for brand-new notes (nothing to delete). Probe (debug build): 3000 notes cold index **14.9 s**; re-index of 3000 edited notes **22.9 s**; ratio is O(N·M). No `rowid` mapping or external-content table. Also per-statement `tx.execute` re-prepares SQL (no `prepare_cached`) ~20× per note.

### M2 — Vector staleness when weights are absent (feature builds)
Changed note with model missing → `sync_note_vectors_tx` returns `Ok("skipped…")` ([vectors/mod.rs#L500-L505](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/vectors/mod.rs#L500-L505)) *before* the `DELETE FROM note_vectors` ([#L512](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/vectors/mod.rs#L512)), yet `file_meta` is committed with the new hash. Old chunks stay; the missing-vector probe ([index/mod.rs#L418-L426](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L418-L426)) keys on `DISTINCT rel_path` presence, so after `setup-models` the edited note is never re-embedded. Semantic search returns stale previews/rankings permanently until content changes again. Static derivation; feature off here.

### M3 — Graph identity collapses on duplicate stems / directory names
- Relations keyed by stem only; `resolve_stem_rel_path` picks `LIMIT 1` without `ORDER BY` ([graph/mod.rs#L74-L86](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/graph/mod.rs#L74-L86)). Probe: `proj/A.md` + `other/A.md` (links `[[A]]`) → `map A` shows `A ↺ (cycle)` under itself (false cycle), upstream of A is `other/A.md` which is also A.
- Downstream LIKE `%/{stem}/%` ([#L104](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/graph/mod.rs#L104), [#L342](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/graph/mod.rs#L342)) matches any *directory* named like the stem. Probe: no `infra.md`, but `proj/infra/z.md` → `blast infra` reports downstream `Q`. Inconsistent: `infra/z.md` (no leading dir) does not match. Over-attribution inflates blast radius.

### M4 — Duplicate relation edges
`relations` has no UNIQUE/dedup ([index/mod.rs#L215-L219](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L215-L219), inserts [#L610-L615](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L610-L615)). Probe: `[[B]]` ×3 in A.md → `blast B` lists `proj/A.md` 3×, `map A` shows B 3×; contract `dependents` likewise. Multiplies traversal fan-out.

### M5 — Shared `visited` truncates diamonds without marking
[graph/mod.rs#L357](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/graph/mod.rs#L357) one `visited` set across siblings. Probe A→B→D→X, A→C→D: `map A --depth 4 --direction down` expands D→X under B only; under C, D is a bare leaf with `cycle=false`. Reader concludes C's chain ends at D. Same in `traverse_up` ([#L397](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/graph/mod.rs#L397)). Terminates, but lies.

### M6 — Parse errors reported once, then lost
Parse errors are collected only for files in `to_update` ([index/mod.rs#L495-L498](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L495-L498)); `file_meta` records the hash, so subsequent syncs show unchanged and `SyncReport.parse_errors` is empty. Probe: `proj/seq.md` (YAML sequence root) was swallowed by a read-path sync; later `reconcile` ×2 print nothing for it (only `lint` shows it). A malformed note is indexed with *whole raw text including frontmatter* as body ([#L497](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L497)) and default title/type — violates DATA_LOSS "raise, don't default".

## Minor (8.5–9.4)
- **Unbounded memory:** every file read fully into `current_map`, cloned to `to_update` ([#L381](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L381)) and again into `vector_sources` ([#L489](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L489), [#L780](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L780)) → ~3× vault size; no file size cap (a 2 GB .md is read whole). `vector_sources.iter().any` in loop ([#L778](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L778)) is O(N²) on first embed.
- **Swallowed row errors:** `rows.flatten()` on `file_meta` ([#L292](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L292)), `walker.flatten()` ([#L313](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L313)) drop permission-denied directories silently → their notes appear *deleted* and are purged from the index (read_errors protect only files that open, not unreadable dirs).
- **Symlinked .md** are not `is_file()` under walkdir defaults ([#L314](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L314)): a note converted to a symlink is deleted from the index.
- **`rel` via `to_string_lossy`** ([#L327](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L327)): two non-UTF8 filenames can collide on U+FFFD keys.
- **Ignored-dir filter on root:** `filter_entry` also tests the vault root ([#L307-L310](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L307-L310)); a vault directory literally named `node_modules`/`_templates` indexes zero notes and deletes the cache.
- **Chunker:** paragraph-less section >1200 chars becomes one window ([vectors/mod.rs#L204-L216](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/vectors/mod.rs#L204-L216)); tokens past 512 are silently cut ([#L441](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/vectors/mod.rs#L441)) so tail text is unsearchable semantically. Regex `Regex::new` compiled per call ([#L132](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/vectors/mod.rs#L132)). Headings level ≥5 not boundaries. Fence close ignores fence length.
- **search_vectors:** loads every blob into a `Vec` before scoring ([#L624-L630](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/vectors/mod.rs#L624-L630)): ~1.5 KB × chunks resident (100k chunks ≈ 150 MB+); wrong-size blob silently `continue` ([#L645-L647](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/vectors/mod.rs#L645-L647)) — silent data loss; NaN score → `Equal` in sort ([#L688](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/vectors/mod.rs#L688)). Model reload (mmap) every query.
- **Graph N+1 / unbounded width:** `resolve_stem_rel_path` + `prepare` per node ([graph/mod.rs#L363](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/graph/mod.rs#L363), [#L343](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/graph/mod.rs#L343)); depth is clamped to 5 ([#L458](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/graph/mod.rs#L458)) but no node cap → fan-out 1000 hub = unbounded output/spill. `ancestors.to_vec()` per node.
- **Fence regex:** unclosed ``` leaves following wikilinks counted ([index/mod.rs#L59](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L59)).
- **Schema ping-pong:** downgrade binary vs newer cache DB drops & rebuilds on every version flip; rebuild is non-transactional.

## Verified-not-defective (counterexample search returned EMPTY)
- SQL construction: all values parameterized; LIKE inputs escaped ([graph/mod.rs#L14-L18](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/graph/mod.rs#L14-L18), `ESCAPE '\'`); no user-string concatenated into SQL. Poison probes (`%`, `_`, `'`) not injectable.
- Blake3 change detection: unchanged files skip (second reconcile `0 updated`); read-error files retained ([#L367](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L367), non-UTF8 `bin.md` stayed indexed-or-absent without purge).
- Cycle termination: `A→A` self/mutual loops terminate via `ancestors` + `visited`.
- Deletions: all 8 derived tables cleaned in delete path ([#L444-L471](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L444-L471)).

## Failure-vector matrix
| Vector | Result |
|---|---|
| Poison payload (non-UTF8 file, YAML sequence root, dotted stem, `%`/`_` in stem) | non-UTF8 contained; dotted-stem → **C1**; seq root → **M6** |
| Concurrency (4× parallel reconcile) | **C3** 3/4 failed `database is locked` |
| Saturation (3000 notes cold/edit) | **M1** 15 s / 23 s quadratic; memory 3× vault |

## Test coverage context
[tests/regression_tests.rs](file:///home/devhax/projects/fusuyfusuy/akatsuki/tests/regression_tests.rs) covers chunk length (L164), malformed FM chunk (L277), read-errors not deleting (L791), missing-vector detection (L862), wikilink fences (L1118). **Uncovered:** concurrent reconcile, dotted-stem blast, duplicate edges, diamond/dup-stem graph, vector tx rollback/staleness, FTS scale, symlink/permission deletion. Vector-path tests need weights (memory.md KNOWN DEBT) so C2/M2 are untested in CI.

## Score rationale: 6.4 — three critical correctness/robustness defects (false-empty blast, concurrency failure, vector-coupled availability); the headline tx tear is fixed.
