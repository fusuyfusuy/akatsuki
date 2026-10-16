---
scope: "Storage & Markdown Engine"
score: 5.6
status: "FAIL — silent data loss in section/frontmatter mutation paths; silent vault-resolution fallback; unbounded lock wait"
critical_findings: 4
invariant_breaches: ["DATA_LOSS (silent fallbacks)", "Frontmatter Strictness (partial parse accepted)", "Heading Normalization single-source (reader/writer split)", "Code Fence Isolation (incomplete)", "contained_path hidden-dir policy bypass"]
---

# Scope 1 — Storage & Markdown Engine Audit (akatsuki v0.2.2 @ fa46b40)

Method: static read of `src/storage/mod.rs` (712L), `constants.rs`, `lib.rs`, `main.rs`, mutation callers; 20 machine probes run against
`target/debug/akatsuki` in `/tmp/akp*` vaults (throwaway script outside repo). Every CRIT/MOD below was reproduced unless marked *static*.

## Critical (score impact < 7)

### C1 — Substring heading fallback makes writers overwrite the wrong section (silent data loss)
- [mod.rs:569](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L569) `scan(exact).or_else(|| scan(contains))` is shared by `replace`/`append` ([mutations/mod.rs:213](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mutations/mod.rs#L213), [:301](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mutations/mod.rs#L301)).
- Counterexample: note has `## Work Logs` (body "precious data"); `akatsuki replace n1 --heading "Log" --content OVERWRITTEN` → exit 0,
  "Section 'Log' replaced successfully", `precious data` count = 0. A typo/short heading destroys an unrelated section; no ambiguity error even when several headings contain the substring (first wins).
- `append` with a non-existent heading is documented to create it ([mutations/mod.rs:313](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mutations/mod.rs#L313)) but substring fallback hijacks it into any heading containing the text.

### C2 — Frontmatter close detection ignores YAML context → metadata silently moved into body
- [mod.rs:292-298](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L292-L298) closes on the first line whose `trim()=="---"`, including indented lines inside block scalars.
- Counterexample: `description: |\n  intro\n  ---\n  tail\nstatus: active` → `set --key owner` exit 0; rewritten note has `status` and `tail` **outside** frontmatter, followed by a stray `---`. The partial parse passes `serde_yaml` (valid prefix), so the "Frontmatter Strictness" gate (memory.md) does not fire. Fix: only accept an unindented `---` (or `...`) at column 0: `line.trim_end_matches(['\r','\n']) == "---"`.

### C3 — `resolve_vault_path` silently falls back to cwd / `$HOME` → mutations land outside any vault
- [mod.rs:88-96](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L88-L96): `AKATSUKI_VAULT` set but not a dir → ignored silently; [mod.rs:156](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L156): final fallback = `current_dir()` (or `"."`). Return type is infallible `PathBuf` — no BoundaryViolation possible.
- Counterexample P9: `AKATSUKI_VAULT=/nonexistent/vault akatsuki write probe-note` in empty dir → exit 0, creates `probe-note.md` **and** `01-Daily/` in cwd.
- Counterexample P10: `$HOME/.akatsuki/` exists (itself a listed default vault, [mod.rs:146](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L146)); upward walk hits `$HOME` first via `.akatsuki` probe ([mod.rs:132](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L132)) → `$HOME` becomes the vault; `write p10` created `$HOME/p10.md`. Any later fs-scan fallback ([mod.rs:455](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L455)) then walks the entire home tree.
- Counterexample P11: `AKATSUKI_VAULT='~/vault'` — `shellexpand` ([mod.rs:159](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L159)) does not expand `~` → falls through → note written into cwd. Also `--vault /typo` returned verbatim ([mod.rs:85](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L85)); `write_atomic` then `create_dir_all`s it.

### C4 — `VaultLock::acquire` blocks unboundedly
- [mod.rs:30](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L30) `lock_exclusive()` with no deadline. Probe P14: external holder of `.akatsuki.lock` for 6s → `timeout 3 akatsuki log …` exit 124. A stuck MCP server (lock is held across full `sync_index`, [mutations/mod.rs:115](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mutations/mod.rs#L115)) or the documented non-reentrant self-deadlock wedges every writer and MCP tool call forever. Fix: `try_lock_exclusive` poll loop with deadline (e.g. 10s, env-tunable) → typed `LockTimeout` error.

## Moderate (7–8.4)

- **M1 Reader/writer section split** — CLI `read --section` and MCP read pass full content incl. frontmatter to `extract_section` ([cli/mod.rs:292](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/cli/mod.rs#L292), [mcp/mod.rs:195](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mcp/mod.rs#L195)); writers pass body only. P5: YAML comment `# Secrets` in frontmatter → `read --section Secrets` returns `token: abc` + `---`; `replace --heading Secrets` errors "not found". Breaches the memory.md "single source of truth" invariant.
- **M2 Fence tracking is not CommonMark** — [mod.rs:533-547](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L533-L547): closer only needs 3-char prefix (no length match, info string allowed). P4: ` ````md ` block containing ` ``` ` → `## Fake` inside the outer fence resolved as a section. Indented code: [mod.rs:504](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L504) `line.trim()` accepts `    # install deps` (4-space code block) as a heading (P3). Writers can therefore split sections inside code.
- **M3 Frontmatter roundtrip is lossy** — `serde_yaml → serde_json::Value → serde_yaml` ([mod.rs:303](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L303), [:341](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L341)). P8 after `set owner`: `ver: 1.10` → `1.1` (version-string corruption), `port: 0x1F` → `31`, all comments (`# keep this comment`, inline) deleted, anchors/aliases expanded; P17 `1: one` → `'1': one`. P7: comment-only frontmatter erased; blank lines after fence eaten ([mod.rs:322](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L322)); P16: CRLF frontmatter rewritten LF (mixed endings in body).
- **M4 `dump_frontmatter` silent fallback** — [mod.rs:341](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L341) `to_string(meta).unwrap_or_default()` → on error emits `---\n---\n` (metadata dropped) instead of `Err`; [mod.rs:338](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L338) non-object meta silently drops fence. *static*. DATA_LOSS breach.
- **M5 Hidden-dir policy bypass via note resolution** — `contained_path` forbids dot-dirs ([mod.rs:228-236](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L228-L236)) but the walk fallback only skips a fixed list ([mod.rs:446-454](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L446-L454)) and its result is never re-validated. P12: `.trash/ghost.md` read **and mutated** (`replace ghost --heading S` exit 0). Same for `.github/`, `.config/` etc.
- **M6 Ambiguous resolution mutates an arbitrary note** — walk returns first readdir hit ([mod.rs:464-476](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L464-L476)); SQL `… OR title = … LIMIT 1` has no `ORDER BY` ([mod.rs:428](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L428)) so a title match can beat an exact stem. P18: `20-Projects/dup.md` + `40-Systems/dup.md` → `set dup` silently wrote `40-Systems/dup.md`; no ambiguity error. Cache opened raw (not `open_synced_db`), so stale projection steers resolution.
- **M7 Token budget bypass** — [mod.rs:681](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L681) `starts_with("---")` treats a leading thematic break / `----` as frontmatter; with no second fence every line is kept. P6: `read --budget 10` (40 chars) returned 140,895 bytes. Frontmatter size itself is uncapped. `budget * 4` overflows (debug panic) for huge MCP `budget` ([mod.rs:674](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L674), *static*). Resource-saturation vector for the 4 KB spill trap.

## Minor (8.5–9.4)

- **m1** `write_atomic` tmp opened without `create_new` (follows pre-planted symlink) and created with umask perms before `set_permissions` → 0600 notes briefly world-readable ([mod.rs:355-376](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L355-L376)); parent dir not fsynced after rename (rename not durable on power loss).
- **m2** `contained_path` check→use TOCTOU: symlink components validated, then path used later without `O_NOFOLLOW`; swap between check and `write_atomic` escapes (*static*, needs local attacker in vault).
- **m3** Leading `---` thematic break makes note unmutable (P20 "fence is never closed") — strict but undocumented; `...` YAML end marker unsupported.
- **m4** `machine_id` spawns `hostname` per mutation when `HOSTNAME` unexported (bash default) and degrades to synthetic `"local"` ([mod.rs:69-77](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L69-L77)); env-file parser ignores inline comments ([mod.rs:105-110](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L105-L110)).
- **m5** `auto_heal_frontmatter` overwrites non-string user values (`title: 2024`, `updated: 1`) because `existing_text` requires `as_str` ([mod.rs:618-623](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L618-L623)); `tags: []` replaced with inferred type.
- **m6** `contained_path` / `ensure_writable_note_path` fall back to `"."` when cwd is unreadable ([mod.rs:208](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L208)); `.akatsuki*` allowed by `contained_path` itself ([mod.rs:231](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L231)) — protection lives only in the mutations-layer guard (P13 blocked there), so any new caller of `contained_path` can write `cache.db` or replace `.akatsuki.lock` (rename swaps inode → breaks mutual exclusion).
- **m7** `is_raw_path` misses `.yml.example`-style? (covered) but `.env.local`, `.service.d` not raw → get `.md` appended (*static*).

## Verified non-defects (counterexample search empty)
- `..` underflow / `a/../.git/hooks` / NUL → rejected ([mod.rs:203-236](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L203-L236)); symlink-to-outside component → `None`.
- `.akatsuki/evil` write blocked by mutation guard (P13). Permission preservation on 0600 target holds (P15). No leftover `*.tmp.*` after normal ops (P19).

## Test-coverage gaps (tests/regression_tests.rs)
Existing: fence comments (L198), exact-over-substring precedence (L473), perms (L670), underflow (L266). Missing: substring-only writer match (C1), block-scalar `---` (C2), vault-resolution failure (C3), lock timeout (C4), frontmatter-comment heading (M1), 4-backtick/indented code (M2), hidden-dir walk (M5), ambiguous stem (M6), hr-prefixed budget (M7).

## Prioritized remediation
1. C1: writers (`replace`/`append`) use exact-match only; substring fallback reader-only and must error on >1 candidate.
2. C2: close fence only on column-0 `---`/`...`; reject trailing content after parse.
3. C3: make `resolve_vault_path` return `Result`; error on explicit/env path not a dir; expand `~`; drop cwd fallback; require vault marker (`40-Systems`+`20-Projects` or `.akatsuki/` **with** `INDEX.md`) and never treat `$HOME` as vault.
4. C4: deadline-bounded `try_lock_exclusive` loop → typed timeout error.
5. M1/M2: route reader through `parse_frontmatter` body; CommonMark fence length/info-string rules; reject ≥4-space-indented headings.
6. M3/M4: preserve raw YAML for untouched keys (line-level patch) or switch to a comment-preserving YAML editor; `dump_frontmatter` → `Result`.
7. M5/M6: re-validate walk hits via `contained_path`; ambiguity → error listing candidates; add `ORDER BY` stem-exact first.
8. M7: detect frontmatter via `opening_fence`; cap frontmatter at budget; `saturating_mul`.
