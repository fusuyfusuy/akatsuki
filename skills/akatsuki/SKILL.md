---
name: akatsuki
description: >
  Bidirectional Knowledge Secretariat gateway to the operator's Phoenix second brain
  (`akatsuki` at `~/configs/knowledge-base/akatsuki`). Use whenever working in any
  repository to (1) read and orient on systems architecture, live services, port mappings,
  project stacks, and cluster policies, or (2) deposit and record architectural decisions,
  infrastructure changes, benchmark results, production deployments, and work logs. Available
  both as a native CLI (`akatsuki`) and as an MCP server (`akatsuki_*` tools).
---

# `akatsuki` // Bidirectional Second Brain Gateway

> *"The Phoenix rises from the ashes of legacy complexity: clean markdown, strict types, zero-jank craft, and autonomous agent symbiosis."*

The **`akatsuki`** skill provides **two-way symbiosis** between autonomous coding agents across all repositories and the operator's central knowledge base at:

```
Vault Root: ~/configs/knowledge-base/akatsuki (or $AKATSUKI_VAULT)
CLI Binary: ~/.local/bin/akatsuki (on PATH)
MCP Server: akatsuki mcp (stdio JSON-RPC 2.0)
```

---

## 🧭 Inbound: Querying & Reading from the Second Brain

When an agent enters or works in ANY project (e.g. `configs/selfhosted`, `3d-filament-finder`, `hepyeni`, `bountools`, `mimori`), use `akatsuki` to instantly understand cluster state, architecture, and constraints:

### 1. Fast CLI Commands

```bash
# Okapi BM25 search with morphological suffix expansion, adaptive boolean query, and snippet extraction
akatsuki search "docker swarm placement"
akatsuki search "postgres database" --domain "40-Systems"

# Extract machine-actionable boundary contract without human narrative bloat (70-90% token reduction)
akatsuki contract 20-Projects/bountools
akatsuki contract Dokploy-Traefik

# Read note with strict token budgeting (prevents context window exhaustion)
akatsuki read Deployment-Playbook --budget 200
akatsuki read Services-Catalog --section "Live Production Services"

# O(1) exact property getter (zero-hallucination structured lookup)
akatsuki get services.bountools.ports
akatsuki get entities.filament.repo
akatsuki get systems.TanriZarAtmaz-Host.status

# Execute read-only SQL against SQLite WAL index tables (entities, services, relations, invariants)
akatsuki query "SELECT name, ports, host FROM services WHERE host = 'TanriZarAtmaz'"

# Calculate architectural blast radius before changing infrastructure or routes
akatsuki blast Dokploy-Traefik
akatsuki blast TanriZarAtmaz

# Execute living invariant assertion blocks (```bash:verify) across notes
akatsuki test
akatsuki test TanriZarAtmaz-Host

# Surgically update a frontmatter key-value property without full-file rewrite
akatsuki set 20-Projects/filament --key status --value maintenance

# Validate that all notes conform to strict machine schemas
akatsuki lint

# Append content directly under a section without full-file rewrite
akatsuki append 20-Projects/my-project --heading "Invariants" --content "- Service timeouts capped at 3s"

# Instant live container & port allocation dump (JSON)
akatsuki services

# Active project inventory and tech stacks (JSON)
akatsuki projects

# View today's operational log & active horizons
akatsuki daily
```

### 2. Available MCP Tools (If MCP is enabled)

- `akatsuki_search(query: str, domain?: str, limit?: int)`: Okapi BM25 search over SQLite FTS5 (`unicode61` tokenizer) with morphological suffix expansion and context snippet excerpts.
- `akatsuki_read(note: str, section?: str, budget?: int)`: Fetches note content or slices a section, with optional token budgeting (`budget`).
- `akatsuki_contract(note: str)`: Slices pure machine-actionable boundary contract (ports, network, relations, invariants, verifications).
- `akatsuki_get(key: str)`: Sub-millisecond O(1) exact property getter (e.g. `services.bountools.ports`, `entities.filament.repo`).
- `akatsuki_query(sql: str)`: Read-only SQL query against index tables (`entities`, `services`, `relations`, `invariants`, `verifications`).
- `akatsuki_blast(target: str)`: Calculates upstream dependents, downstream dependencies, and boundary sinks for a target.
- `akatsuki_test(note?: str)`: Executes machine-verifiable assertion blocks (`bash:verify`) against the host.
- `akatsuki_set(note: str, key: str, value: str)`: Surgically updates frontmatter key-values under kernel lock.
- `akatsuki_lint()`: Validates all notes against strict type schemas.
- `akatsuki_append_section(note: str, heading: str, content: str)`: Atomically injects markdown content under a heading.
- `akatsuki_write_note(path: str, content: str, overwrite?: bool)`: Writes/updates a note with auto-healed YAML frontmatter.
- `akatsuki_services()`: Retrieves active Docker Swarm containers, replicas, ports, and roles as structured JSON.
- `akatsuki_projects()`: Dumps registered software projects and stacks as structured JSON.
- `akatsuki_daily(date?: str)`: Retrieves today's active focus horizon and recent entries.
- `akatsuki_record_log(project: str, summary: str, device?: str)`: Appends timestamped log entry tagged with device into today's daily note under kernel lock.
- `akatsuki_verify()`: Verifies wikilink integrity across the entire vault.
- `akatsuki_list_notes(domain?: str)`: Lists notes and summaries filtered by domain.

---

## ⚡ Outbound: Depositing & Recording into the Second Brain

Whenever you complete work, alter system contracts, or ship a deployment, file it directly back into `akatsuki`:

### 1. Work Log Capture (Telegraphic Caveman Syntax)

```bash
akatsuki log --project "<project-name>" --summary "<verb> <target> -> <delta>; <evidence>" [--device "<device>"]
```

*Enforces telegraphic caveman syntax (<140 chars): omit articles, copulas, and filler. Automatically resolves host machine if `--device` is omitted. Appends `- **HH:MM** [device]: [project] <summary>` to today's `01-Daily/YYYY-MM-DD.md`.*

Mutations made via `write`, `set`, and `append` automatically update `updated` and `updated_by` frontmatter attributes and record a mutation audit entry into today's daily log.

### 2. Updating Project or System Specs

- When an agent updates a project's architecture, routes, or database schemas:
  Edit `~/configs/knowledge-base/akatsuki/20-Projects/<project-name>.md`.
- When an agent updates infrastructure (Docker Swarm, EPYC tuning, runner nodes):
  Edit `~/configs/knowledge-base/akatsuki/40-Systems/<system-name>.md`.

### 3. Onboarding a New Service

1. Copy `~/configs/knowledge-base/akatsuki/_templates/project-template.md` to `20-Projects/<new-project>.md`.
2. Populate frontmatter, architecture, invariants, and roadmap.
3. Register wikilink in `20-Projects/Projects-MOC.md` and `INDEX.md`.

---

## 🏛️ Secretariat Guardrails & Rules Enforced

1. **Pure Markdown & Zero Plugin Lock-In**:
   - **MUST NOT emit Obsidian plugin codeblocks**: No ````tasks````, no ````dataviewjs````, no Templater `<% ... %>`.
   - All tasks must be standard markdown checkboxes (`- [ ]`, `- [x]`).
2. **Valid YAML Frontmatter**:
   - Every file must have metadata: `title`, `date`, `type`, `tags`, `summary`.
3. **Wikilinks & Zero Orphans**:
   - Use standard Obsidian wikilinks: `[[Domain/NoteName|Display Text]]` or `[[NoteName]]`.
   - Every note MUST be linked to its parent MOC and/or `INDEX.md`.
4. **Facts Over Prose**:
   - Cite exact file paths, commit hashes, machine names (`TanriZarAtmaz`, `OCocuk`, `HakimBey`), port numbers, and exit codes.

---

## 🧪 Verification Gate

Before concluding any session that modified `akatsuki`:

```bash
akatsuki verify
```

Exit code MUST be 0 (`PASSED: All wikilinks in akatsuki resolve cleanly.`).
