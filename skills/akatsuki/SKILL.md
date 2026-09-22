---
name: akatsuki
description: >
  Bidirectional Knowledge Secretariat gateway to the operator's Phoenix second brain.
  Use when entering any repository to orient on systems architecture, live services, port mappings,
  project stacks, and cluster policies, or to deposit architectural decisions, production deployments,
  and telegraphic work logs. Native CLI and MCP stdio.
---

# AKATSUKI(1) — Knowledge Secretariat & Second Brain Gateway

```text
KERNEL:
  TARGET: Autonomous Agent Knowledge Secretariat & Cross-Project Topology
  BINARY: akatsuki (CLI) | akatsuki mcp (JSON-RPC 2.0 stdio)
  VAULT:  ~/configs/knowledge-base/akatsuki (or $AKATSUKI_VAULT, or --vault <dir>)
  STORAGE: Markdown (source of truth) + .akatsuki/cache.db (derived projection, SQLite WAL)
  INVARIANTS:
    1_TELEMETRY:  Telegraphic Caveman (<= 280 chars soft limit; No articles/copulas/pronouns)
    2_PURITY:     Pure Markdown & Strict YAML — zero Obsidian plugin lock-in
    3_BUDGETING:  Token-Bounded Reads (contract > read --budget > full read)
    4_INTEGRITY:  Zero Orphan Notes — wikilink resolution verify == exit 0
    5_VECTORS:    Optional Candle engine (`--features vectors`): intfloat/multilingual-e5-small in
                  ~/.cache/akatsuki/models/, fused with BM25 via RRF. Absent weights or a default
                  build degrade to keyword-only search WITH an explicit notice, never a silent swap.
```

## SYNOPSIS

```shell
akatsuki search   <query> [--domain <d>] [--limit <N>] [--mode hybrid|bm25|vector] [--with-graph] [--compact] [--json]
akatsuki contract <note> [--json]
akatsuki read     <note> [--section <sec>] [--budget <N>] [--json]     # alias: cat
akatsuki get      <key> [--json]
akatsuki query    <sql> [--json]
akatsuki blast    <target> [--json]
akatsuki map      <target> [--depth <N>] [--direction up|down|both] [--json]
akatsuki test     [<note>] [--dry-run] [--json]
akatsuki set      <note> --key <key> --value <val> [--json]
akatsuki append   <note> --heading <heading> --content <content> [--json]
akatsuki replace  <note> --heading <heading> --content <content> [--json]
akatsuki write    <path> --content <content> [--overwrite] [--raw] [--json]
akatsuki services [--json]
akatsuki projects [--json]
akatsuki daily    [--date <YYYY-MM-DD>] [--json]
akatsuki log      --project <proj> --summary <sum> [--device <dev>] [--json]
akatsuki list     [--domain <d>] [--json]                              # alias: ls
akatsuki lint     [--json]
akatsuki verify   [--json]
akatsuki reconcile [--dry-run] [--json]
akatsuki setup-models
akatsuki mcp      [--vault <dir>]
```

`--content` accepts values starting with `-` (pass markdown bullets literally).
`--mode` is validated: an unknown mode is a CLI error (exit 2), never a silent fallback.

---

## AGENT LIFECYCLE PIPELINE

```text
ORIENT -> CONTRACT -> MAP/BLAST -> MUTATE -> VERIFY -> LOG
```

1. **TURN-0 (Inbound Orientation)**:
   - Live Topology & Ports: `akatsuki services` | MCP: `akatsuki_services()`
   - Project Stacks: `akatsuki projects` | MCP: `akatsuki_projects()`
   - Knowledge Search: `akatsuki search "<topic>" --with-graph` | MCP: `akatsuki_search(query="<topic>", with_graph=True)`
2. **CONTRACT (Token-Dense Slicing)**:
   - `akatsuki contract <note>` | MCP: `akatsuki_contract(note="<note>")`
   - Returns ports, relations, invariants, verifications, dependencies and dependents — 70–90% fewer tokens than a full read.
3. **MAP / BLAST (Pre-Mutation Safety Gate)**:
   - Recursive Map: `akatsuki map <target> [--depth 2]` → ASCII tree by default, JSON with `--json`.
   - Blast Radius: `akatsuki blast <target>` → `upstream[]` (`source_rel`), `downstream[]` (`target_stem`), `boundary_sinks[]`.
4. **MUTATE (Structured Updates)**:
   - Property: `akatsuki set <note> --key owner.name --value yusuf` (dotted paths nest; `[1,2]`/`true`/`42` are typed)
   - Section append: `akatsuki append <note> --heading H --content "<markdown>"` (creates the note and/or heading when missing)
   - Section replace: `akatsuki replace <note> --heading H --content "<markdown>"` (heading line preserved)
   - Note creation: `akatsuki write <path> --content C` (markdown frontmatter is completed to satisfy lint)
   - Heading matching is emoji/punctuation tolerant: `--heading Overview` addresses `## 📌 Overview`.
   - Every mutation stamps `updated` + `updated_by` and appends an audit line to today's daily note.
5. **VERIFY (Integrity Gate)**:
   - `akatsuki lint ∧ akatsuki verify == exit 0`
   - Wikilinks and markdown links both resolve; every domain note is indexed by its MOC or `INDEX.md`.
6. **LOG (Outbound Telemetry)**:
   - `akatsuki log --project <proj> --summary "<verb> <target> -> <delta>; <evidence>"`
   - Appends chronologically to today's daily note under the vault lock; device = `--device` > `$AKATSUKI_HOST` > `$HOSTNAME` > `hostname -s`.

---

## SUBCOMMAND SPECIFICATIONS

### `search` — Hybrid BM25 & Dense Vector Search
- **Retrieval Modes** (`--mode`, `-m`):
  - `hybrid` (default): Okapi BM25 fused with dense embeddings via Reciprocal Rank Fusion ($k=60$) **when the binary was built with `--features vectors` and the weights are present**. Otherwise BM25, and the output carries an explicit `⚠ semantic ranking unavailable…` line.
  - `bm25`: SQLite FTS5, `unicode61 remove_diacritics 2`, column weights (title 10 / tags 5 / summary 5 / body 1), prefix + suffix-stem term expansion (`caching` reaches `cache`).
  - `vector`: pure cosine over 384-dimensional e5 vectors. Errors (exit 1) if the feature or the weights are missing — it never degrades silently.
- **Freshness**: every query reconciles the projection first; a vault that was never reconciled still answers correctly.
- `--domain <d>` / `--limit <N>` (default 10; MCP default 5) / `--with-graph` attaches 1-hop relations and container/port allocations.
- Vector index maintenance is incremental: only notes whose Blake3 content hash changed are re-embedded.

### `contract` — Boundary Contract Slicing
`akatsuki contract <note>` extracts machine-actionable interfaces: `note_type`, declared ports, `relations[]`, `invariants[]`, `verifications[]`, `dependencies[]`, `dependents[]`. Primary orientation primitive — use instead of `read`.

### `read` — Token-Bounded Note Reading
`akatsuki read <note> [--section <sec>] [--budget <N>]`: full note or one section; `--budget` packs to roughly $N$ tokens (~4 chars each), keeps the frontmatter block intact, and appends an explicit truncation notice.

### `get` & `query` — Structured Data Extraction
- `get <keypath>`: O(1) dotted lookup. `services.api.ports` reads the services projection; `entities.<stem>.<field>` reads that note's frontmatter; otherwise the keypath is walked over a note's frontmatter.
- `query <sql>`: read-only SQL (`SELECT`/`WITH`/`EXPLAIN`) over `notes_fts`, `entities`, `services`, `relations`, `invariants`, `verifications`, `note_vectors`. Writes are rejected.

### `blast` / `map` — Dependency & Traversal
- `blast <target>`: upstream dependents, downstream dependencies, boundary sinks (container, ports, host, network).
- `map <target> [--depth 1..5] [--direction up|down|both]`: ASCII tree with `[relation]` prefixes and ` ↺ (cycle)` markers; `--json` emits `{target, rel_path, depth, direction, downstream[], upstream[], boundary_sinks[]}` where each node is `{stem, rel_path, rel_type, cycle, children[]}`.

### `test` — Invariant Verification Runner
- Extracts and executes ` ```bash:verify ` blocks against live infrastructure with the vault as cwd.
- Each assertion has a hard ceiling (default 10s, override `AKATSUKI_INVARIANT_TIMEOUT`); a hang is reported as exit 124 with the process group killed.
- `--dry-run` lists the assertions and marks them `[DRY-RUN - not executed]`; it never executes.
- `test <note>` with a filter that matches nothing is an error, not a green result.

### `set`, `append`, `replace` & `write` — Structured Mutations
- `set` walks dotted keypaths and infers types (integer, boolean, JSON array/object, else string).
- `append` inserts markdown verbatim at the end of the section, creating the note and/or the heading when absent.
- `replace` swaps a section's contents and keeps the heading line.
- `write` requires a vault-relative path (absolute paths are rejected) and completes markdown frontmatter (`title`, `date`, `type`, `tags`, `summary`, `updated`, `updated_by`; `status: active` for projects) unless `--raw`.
- All mutations run under one advisory vault lock, write via tmp-file + rename, and log `audit` lines. `updated` / `updated_by` are stamped on every one.

### `services` & `projects` — Live Topology Dumps
- `services`: container prefixes, replicas, ports, roles, hosts, networks — from the `Services-Catalog.md` table, `services:` frontmatter blocks, or notes typed as services.
- `projects`: `20-Projects` notes with repository, host, network and deployment status.

### `daily` & `log` — Operational Telemetry Ledger
- `daily [--date YYYY-MM-DD]`: today's (or the given date's) note; a missing date returns a soft "does not exist yet" message. Dates are validated — traversal is rejected.
- `log`: appends `- **HH:MM** [device]: [project] summary` chronologically.

### `list`, `lint` & `verify` — Vault Health Gates
- `list [--domain d]`: `rel_path`, `stem`, `title`, `type`, `summary`, `status`, `tags`.
- `lint`: strict YAML plus per-type required fields (`project` also needs `status`; every type needs `tags`), and syntax checks for raw assets under `50-Configs/` and `60-Scripts/` (`.yaml`, `.json`, `.sh`). `bash:verify` blocks that invoke repo unit tests are a boundary violation.
- `verify`: broken wikilinks, broken markdown links, orphan notes, and notes a domain MOC fails to index.

### `reconcile` — Projection Rebuild & Repair
- Syncs the projection (Blake3 content hashes, Rayon-parallel scan), auto-quotes frontmatter scalars containing `": "`, appends unindexed notes to their domain MOC, and embeds changed notes when the vector feature is available.
- `--dry-run` reports every action it would take and writes nothing (including no index mutation).
- Notes whose frontmatter cannot be parsed are indexed without metadata and reported as `unindexable frontmatter` errors — never silently dropped.

---

## MCP SERVER & TOOL DUALITY

Run stdio daemon: `akatsuki mcp [--vault <dir>]`. Every tool call is panic-isolated: one failing tool returns `isError`, it never kills the server. Messages without an `id` are notifications and receive no reply.

| MCP Tool | CLI Equivalent | Key Arguments |
| :--- | :--- | :--- |
| `akatsuki_search` | `akatsuki search` | `query`, `mode`, `domain`, `limit`, `with_graph` |
| `akatsuki_read` | `akatsuki read` | `note` (or `path`), `section`, `budget` |
| `akatsuki_contract` | `akatsuki contract` | `note` (or `target`, `path`) |
| `akatsuki_blast` | `akatsuki blast` | `target` (or `note`, `path`) |
| `akatsuki_map` | `akatsuki map` | `target` (or `note`, `path`), `depth`, `direction` |
| `akatsuki_services` | `akatsuki services` | *(none)* |
| `akatsuki_projects` | `akatsuki projects` | *(none)* |
| `akatsuki_record_log` | `akatsuki log` | `project`, `summary`, `device` |
| `akatsuki_write_note` | `akatsuki write` | `path`, `content`, `overwrite`, `raw` |
| `akatsuki_get` | `akatsuki get` | `key` (or `keypath`) |
| `akatsuki_query` | `akatsuki query` | `sql` |
| `akatsuki_set` | `akatsuki set` | `note`, `key`, `value` |
| `akatsuki_append_section` | `akatsuki append` | `note`, `heading`, `content` |
| `akatsuki_replace_section` | `akatsuki replace` | `note`, `heading`, `content` |
| `akatsuki_daily` | `akatsuki daily` | `date` |
| `akatsuki_lint` | `akatsuki lint` | *(none)* |
| `akatsuki_verify` | `akatsuki verify` | *(none)* |
| `akatsuki_test` | `akatsuki test` | `note`, `dry_run` |
| `akatsuki_reconcile` | `akatsuki reconcile` | `dry_run` |
| `akatsuki_list_notes` | `akatsuki list` | `domain` |

Booleans and numbers are accepted in either JSON or string form (`"dry_run": "true"` behaves as `true`).
No MCP resource layer is implemented: `resources/*` returns `-32601`.

---

## TELEMETRY & VAULT CONTRACTS

### 1. Telegraphic Log Format Contract
```text
[proj] <verb> <target> -> <delta>; <evidence/exit>
```
* **Rules**: Soft limit ≤ 280 chars. Omit articles (`a`, `an`, `the`), copulas (`is`, `was`), and pronouns (`I`, `we`).
* **Example**: `[dokploy] update traefik-cert -> renew wildcard SAN; exit 0`

### 2. Obsidian Compatibility & Zero-Plugin Invariant
- **Banned Blocks**: NEVER emit ````tasks````, ````dataviewjs````, or `<% templater %>` tags.
- **Checkboxes**: Use standard markdown checkboxes only (`- [ ]`, `- [x]`).
- **Wikilinks**: Always link notes with standard syntax: `[[TargetNote]]` or `[[TargetNote|Alias]]`. Zero orphan notes permitted.

### 3. Frontmatter Contract
- Every note carries `title`, `date`, `type`, `tags`, `summary` (`project` notes also `status`).
- `type` is inferred from the domain directory on write (`20-Projects` → project, `40-Systems` → system, …).
- Malformed YAML is a hard error for `lint` and for any mutation of that note: metadata is never rewritten from a partial parse.

---

## EXIT CODES

- `0`: Success / Verification Passed.
- `1`: Unresolved link, schema lint error, failed invariant assertion, refused mutation, or note not found.
- `2`: Invalid CLI arguments.
