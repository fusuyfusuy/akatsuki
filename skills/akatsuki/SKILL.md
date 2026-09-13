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
  BINARY: ~/.local/bin/akatsuki (CLI) | akatsuki mcp (JSON-RPC stdio)
  VAULT:  ~/configs/knowledge-base/akatsuki (or $AKATSUKI_VAULT)
  INVARIANTS:
    1_TELEMETRY:  Telegraphic Caveman (<140 chars; No articles/copulas/pronouns)
    2_PURITY:     Pure Markdown & Strict YAML — zero Obsidian plugin lock-in
    3_BUDGETING:  Token-Bounded Reads (contract > read --budget > full read)
    4_INTEGRITY:  Zero Orphan Notes — wikilink resolution verify == exit 0
```

## SYNOPSIS

```shell
akatsuki search   <query> [--domain <domain>] [--limit <N>] [--json]
akatsuki contract <note> [--json]
akatsuki read     <note> [--section <sec>] [--budget <N>] [--json]
akatsuki get      <key> [--json]
akatsuki query    <sql> [--json]
akatsuki blast    <target> [--json]
akatsuki test     [<note>] [--json]
akatsuki set      <note> --key <key> --value <val> [--json]
akatsuki append   <note> --heading <heading> --content <content> [--json]
akatsuki write    <path> --content <content> [--overwrite] [--json]
akatsuki services [--json]
akatsuki projects [--json]
akatsuki daily    [--date <YYYY-MM-DD>] [--json]
akatsuki log      --project <proj> --summary <sum> [--device <dev>]
akatsuki lint     [--json]
akatsuki verify   [--json]
akatsuki reconcile [--dry-run]
akatsuki mcp      [--vault <dir>]
```

---

## AGENT LIFECYCLE PIPELINE

```text
ORIENT -> CONTRACT -> BLAST -> MUTATE -> VERIFY -> LOG
```

1. **TURN-0 (Inbound Orientation)**:
   - Live Topology & Ports: `akatsuki services` | MCP: `akatsuki_services()`
   - Project Stacks: `akatsuki projects` | MCP: `akatsuki_projects()`
   - Knowledge Search: `akatsuki search "<topic>"` | MCP: `akatsuki_search(query="<topic>")`
2. **CONTRACT (Token-Dense Slicing)**:
   - `akatsuki contract <note>` | MCP: `akatsuki_contract(note="<note>")`
   - Slices pure machine-actionable boundaries (ports, network, relations, invariants, verifications), eliminating 70–90% narrative token bloat.
3. **BLAST (Pre-Mutation Safety Gate)**:
   - `akatsuki blast <target>` | MCP: `akatsuki_blast(target="<target>")`
   - Maps upstream dependents, downstream dependencies, and boundary sinks across infrastructure and services before applying changes.
4. **MUTATE (Structured Updates)**:
   - Property update: `akatsuki set <note> --key K --value V` | MCP: `akatsuki_set(note, key, value)`
   - Section append: `akatsuki append <note> --heading H --content C` | MCP: `akatsuki_append_section(note, heading, content)`
   - Note creation: `akatsuki write <path> --content C` | MCP: `akatsuki_write_note(path, content)`
5. **VERIFY (Integrity Gate)**:
   - `akatsuki lint ∧ akatsuki verify == exit 0`
   - Enforces strict schema conformance, valid frontmatter, and bidirectional wikilink closure across the entire vault.
6. **LOG (Outbound Telemetry)**:
   - `akatsuki log --project <proj> --summary "<verb> <target> -> <delta>; <evidence>"`
   - Enforces telegraphic caveman syntax (<140 chars). Injects timestamped entry into today's daily log under kernel lock.

---

## SUBCOMMAND SPECIFICATIONS

### `search` — Okapi BM25 Knowledge Search
```shell
akatsuki search <query> [--domain <domain>] [--limit <N>] [--json]
```
- Full-text search over SQLite FTS5 index with `unicode61` tokenizer and morphological suffix expansion.
- `--domain <dir>`: Restrict search (e.g. `20-Projects`, `40-Systems`).
- `--limit <N>`: Truncate matches to fit token budgets (default: 10).

### `contract` — Boundary Contract Slicing
```shell
akatsuki contract <note> [--json]
```
- Extracts machine-actionable interfaces: declared ports, network bindings, dependencies, invariants, and live verification blocks.
- Primary orientation primitive: use instead of `read` to save 70–90% context tokens.

### `read` — Token-Bounded Note Reading
```shell
akatsuki read <note> [--section <sec>] [--budget <N>] [--json]
```
- Fetches full note content or surgically slices a specific markdown section.
- `--budget <N>`: Truncates content cleanly to fit within token bounds.

### `get` & `query` — Structured Data Extraction
```shell
akatsuki get <key> [--json]    # Sub-millisecond O(1) exact property getter
akatsuki query <sql> [--json]  # Read-only SQL query against SQLite index
```
- `get`: Fast dot-path access (e.g. `services.bountools.ports`, `entities.filament.repo`, `systems.TanriZarAtmaz-Host.status`).
- `query`: Direct SQL querying against index tables (`entities`, `services`, `relations`, `invariants`, `verifications`).

### `blast` — Dependency & Ripple Analysis
```shell
akatsuki blast <target> [--json]
```
- Calculates upstream dependents, downstream dependencies, and boundary sinks for services, containers, or host nodes.

### `test` — Invariant Verification Runner
```shell
akatsuki test [<note>] [--json]
```
- Extracts and executes embedded executable invariant blocks (` ```bash:verify `) against live infrastructure.

### `set`, `append` & `write` — Structured Mutations
```shell
akatsuki set <note> --key <key> --value <val> [--json]
akatsuki append <note> --heading <heading> --content <content> [--json]
akatsuki write <path> --content <content> [--overwrite] [--json]
```
- `set`: Surgically modifies frontmatter keys without rewriting file bodies.
- `append`: Atomically injects markdown bullets or text under a specific heading.
- `write`: Creates or updates a note with auto-healed YAML frontmatter.
- All mutations automatically update `updated` timestamps and log audit trails into today's daily log.

### `services` & `projects` — Live Topology Dumps
```shell
akatsuki services [--json]  # Active Docker Swarm containers, replicas, ports, and roles
akatsuki projects [--json]  # Registered software projects, repositories, and tech stacks
```

### `daily` & `log` — Operational Telemetry Ledger
```shell
akatsuki daily [--date <YYYY-MM-DD>] [--json]
akatsuki log --project <proj> --summary <sum> [--device <dev>]
```
- `daily`: Returns today's active focus horizon and recent work entries.
- `log`: Appends telegraphic work log (`- **HH:MM** [device]: [project] <summary>`). Automatically detects local hostname if `--device` is omitted.

### `lint` & `verify` — Vault Health Gates
```shell
akatsuki lint [--json]    # Validates note frontmatter and schemas
akatsuki verify [--json]  # Asserts zero broken wikilinks across entire vault
```

---

## MCP SERVER & TOOL DUALITY

Run stdio daemon: `akatsuki mcp [--vault <dir>]`

| MCP Tool | CLI Equivalent | Key Arguments |
| :--- | :--- | :--- |
| `akatsuki_search` | `akatsuki search` | `query`, `domain`, `limit` |
| `akatsuki_contract` | `akatsuki contract` | `note` |
| `akatsuki_read` | `akatsuki read` | `note`, `section`, `budget` |
| `akatsuki_get` | `akatsuki get` | `key` |
| `akatsuki_query` | `akatsuki query` | `sql` |
| `akatsuki_blast` | `akatsuki blast` | `target` |
| `akatsuki_test` | `akatsuki test` | `note` |
| `akatsuki_set` | `akatsuki set` | `note`, `key`, `value` |
| `akatsuki_append_section` | `akatsuki append` | `note`, `heading`, `content` |
| `akatsuki_write_note` | `akatsuki write` | `path`, `content`, `overwrite` |
| `akatsuki_services` | `akatsuki services`| *(none)* |
| `akatsuki_projects` | `akatsuki projects`| *(none)* |
| `akatsuki_daily` | `akatsuki daily` | `date` |
| `akatsuki_record_log` | `akatsuki log` | `project`, `summary`, `device` |
| `akatsuki_lint` | `akatsuki lint` | *(none)* |
| `akatsuki_verify` | `akatsuki verify` | *(none)* |
| `akatsuki_list_notes` | *(internal)* | `domain` |

---

## TELEMETRY & VAULT CONTRACTS

### 1. Telegraphic Log Format Contract
Work log summaries MUST follow this exact schema:
```text
[proj] <verb> <target> -> <delta>; <evidence/exit>
```
* **Rules**: Strictly $<140$ chars. Omit articles (`a`, `an`, `the`), copulas (`is`, `was`), and pronouns (`I`, `we`).
* **Example**: `[dokploy] update traefik-cert -> renew wildcard SAN; exit 0`
* **Example**: `[filament] patch scraper -> fix selector drift on product grid; smoke test pass`

### 2. Obsidian Compatibility & Zero-Plugin Invariant
- **Banned Blocks**: NEVER emit ````tasks````, ````dataviewjs````, or `<% templater %>` tags.
- **Checkboxes**: Use standard markdown checkboxes only (`- [ ]`, `- [x]`).
- **Wikilinks**: Always link notes with standard syntax: `[[TargetNote]]` or `[[TargetNote|Alias]]`. Zero orphan notes permitted.

---

## EXIT CODES

- `0`: Success / Verification Passed.
- `1`: Unresolved wikilink, schema lint error, failed invariant assertion, or note not found.
- `2`: Invalid CLI arguments.
