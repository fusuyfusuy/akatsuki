# 🌅 Akatsuki (暁)

> **High-Performance Native Rust CLI and Model Context Protocol (MCP) Knowledge Secretariat for Living System Memory, Architectural Contracts, and Multi-Agent Orchestration.**

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust: 2021](https://img.shields.io/badge/Rust-2021%20Edition-orange.svg)](https://www.rust-lang.org/)
[![MCP: 2024-11-05](https://img.shields.io/badge/MCP-2024--11--05-blue.svg)](https://modelcontextprotocol.io/)
[![SQLite: WAL + FTS5](https://img.shields.io/badge/SQLite-WAL%20%2B%20FTS5-lightgrey.svg)](https://www.sqlite.org/)
[![Candle: Embeddings](https://img.shields.io/badge/Candle-intfloat%2Fe5--small-brightgreen.svg)](https://github.com/huggingface/candle)

---

## 🏛️ What is Akatsuki?

As AI coding swarms and autonomous agents (`Antigravity`, `Claude Code`, `pi`, `OpenCode`, `Cursor`) write and refactor code, traditional static documentation decays. Infrastructure configurations drift, network ports clash, and architectural boundaries get silently breached.

**Akatsuki** solves this by providing a **Living Systems Memory Substrate**:
- **Dual-Layer Architecture**: Pure Markdown Git repository as the immutable source of truth paired with an ephemeral, high-speed SQLite WAL projection (`.akatsuki/cache.db`) synced via parallel Blake3 content hashing (Rayon).
- **Machine-Verifiable Contracts**: Inspect living system contracts, service boundaries, and cluster topology (`akatsuki contract <note>`) using 70–90% fewer tokens than raw document reads.
- **Living Invariant Testing (`bash:verify`)**: Execute machine-verifiable infrastructure assertions (ports, containers, daemons) embedded in Markdown notes with process group isolation, execution timeouts, and security sanitization.
- **Hybrid Search (BM25 + Semantic Vectors)**: Fuses SQLite FTS5 Okapi BM25 with pure-Rust Candle embeddings (`intfloat/multilingual-e5-small`, 384D) via Reciprocal Rank Fusion (RRF, $k=60$).
- **Native Dual-Interface (CLI + MCP)**: Operate seamlessly from the human terminal (`akatsuki <command>`) or equip autonomous agents directly through native JSON-RPC 2.0 stdio (`akatsuki mcp`).
- **Resilient MCP Engine**: 20 specialized architectural tools with panic isolation (`catch_unwind`), type coercion (`arg_bool`, `arg_usize`), and parameter aliasing.

---

## 🚀 Quick Start

### 1. Installation

**Option A: Standard Installation (Pure Lexical BM25, Zero ML Overhead)**
Compiles with native SQLite FTS5 lexical ranking, instant sub-millisecond startup:
```bash
cargo install --path .
```

**Option B: With Semantic Vector Embeddings & Hybrid RRF Search**
Compiles with the pure-Rust Candle tensor engine for local offline semantic embeddings:
```bash
# Build and install with vector features
cargo install --path . --features vectors

# Download the multilingual-e5-small model weights into ~/.cache/akatsuki/models/
akatsuki setup-models
```

### 2. Living Memory Vault Structure

Akatsuki operates on structured knowledge vaults:
```text
knowledge-base/
├── .akatsuki/          # Derived SQLite projection and locks (gitignored, auto-rebuilt)
├── .gitignore          # Multi-machine ignore rules
├── AGENTS.md           # Master architectural protocol and invariants
├── INDEX.md            # Auto-maintained catalog and domain index
├── 01-Daily/           # Daily activity ledgers & worklogs
├── 20-Projects/        # Active software project architectures
├── 40-Systems/         # Host specifications, topologies & ADRs
└── _templates/         # Note, system, and daily templates
```

### 3. Basic CLI Commands

```bash
# Hybrid Search (Okapi BM25 + Dense Semantic Vectors via RRF, k=60)
akatsuki search "docker swarm routing" --mode hybrid --with-graph

# Pure lexical BM25 search
akatsuki search "docker swarm routing" --mode bm25

# Pure semantic vector search (errors explicitly if weights are absent)
akatsuki search "hardware specifications of primary host" --mode vector

# Machine boundary contract extraction (ports, invariants, relations, dependents)
akatsuki contract 40-Systems/Cluster-Topology.md

# Token-budgeted surgical note read
akatsuki read "Cluster-Topology" --section "Private Network Routing" --budget 500

# Graph traversal and dependency blast radius
akatsuki map auth-service --depth 2 --direction both
akatsuki blast auth-service

# Run living invariant checks across system documentation
akatsuki test

# Record an outbound telegraphic caveman ledger entry (<= 280 chars)
akatsuki log --project "web" --summary "migrate edge certs -> renew wildcard SAN; exit 0"

# Surgically update frontmatter without corrupting note bodies
akatsuki set "20-Projects/filament" --key status --value "live"

# Verify vault integrity (wikilinks, orphans, MOC reachability) and lint YAML
akatsuki verify
akatsuki lint

# Synchronize SQLite projection with vault disk state
akatsuki reconcile
```

---

## 🤖 MCP Server Setup (for AI Agents)

Akatsuki runs as a native stdio Model Context Protocol (MCP) server exposing 20 specialized architectural tools:

```bash
akatsuki mcp [--vault <dir>]
```

### Configuration Snippets

#### Claude Desktop (`claude_desktop_config.json`)
```json
{
  "mcpServers": {
    "akatsuki": {
      "command": "akatsuki",
      "args": ["mcp"],
      "env": {
        "AKATSUKI_VAULT": "/path/to/your/knowledge-base"
      }
    }
  }
}
```

#### Cursor (`.cursor/mcp.json`)
```json
{
  "mcpServers": {
    "akatsuki": {
      "command": "akatsuki",
      "args": ["mcp"]
    }
  }
}
```

#### Antigravity (`~/.gemini/antigravity-cli/mcp/`) / OpenCode / pi
```json
{
  "mcpServers": {
    "akatsuki": {
      "command": "akatsuki",
      "args": ["mcp"]
    }
  }
}
```

---

## 🛠️ Complete MCP Tools Reference (20 Tools)

| MCP Tool | CLI Equivalent | Key Arguments | Description |
| :--- | :--- | :--- | :--- |
| `akatsuki_search` | `akatsuki search` | `query`, `mode`, `domain`, `limit`, `with_graph` | Fast hybrid/BM25 search with contextual snippets and optional graph context. |
| `akatsuki_read` | `akatsuki read` | `note` (or `path`), `section`, `budget` | Read full notes or extract individual headings with token budget packing. |
| `akatsuki_contract` | `akatsuki contract` | `note` (or `target`, `path`) | Extract machine boundary contracts (APIs, ports, schemas, invariants). |
| `akatsuki_blast` | `akatsuki blast` | `target` (or `note`, `path`) | Compute upstream callers and downstream dependencies for blast radius analysis. |
| `akatsuki_map` | `akatsuki map` | `target`, `depth`, `direction` | Hierarchical knowledge graph traversal (`up`, `down`, or `both`). |
| `akatsuki_services` | `akatsuki services` | *(none)* | Read active service matrices, container prefixes, and port allocations. |
| `akatsuki_projects` | `akatsuki projects` | *(none)* | List all tracked software repositories and production deployment statuses. |
| `akatsuki_record_log` | `akatsuki log` | `project`, `summary`, `device` | Append telegraphic ledger entries to today's active worklog under kernel lock. |
| `akatsuki_write_note` | `akatsuki write` | `path`, `content`, `overwrite`, `raw` | Create or replace vault notes with auto-completed frontmatter. |
| `akatsuki_get` | `akatsuki get` | `key` (or `keypath`) | $O(1)$ exact metadata lookup across frontmatter and projection tables. |
| `akatsuki_query` | `akatsuki query` | `sql` | Read-only SQL queries (`SELECT`/`WITH`) against SQLite projection tables. |
| `akatsuki_set` | `akatsuki set` | `note`, `key`, `value` | Surgically update YAML frontmatter keys with JSON/primitive typed values. |
| `akatsuki_replace_section`| `akatsuki replace` | `note`, `heading`, `content` | Surgically replace section contents while preserving the heading line. |
| `akatsuki_append_section` | `akatsuki append` | `note`, `heading`, `content` | Append markdown bullets or text under a specific heading. |
| `akatsuki_daily` | `akatsuki daily` | `date` | Read today's or specified daily horizon and work ledger. |
| `akatsuki_lint` | `akatsuki lint` | *(none)* | Validate vault notes against strict YAML schemas and boundary rules. |
| `akatsuki_verify` | `akatsuki verify` | *(none)* | Verify bidirectional wikilink closure, markdown links, and MOC reachability. |
| `akatsuki_test` | `akatsuki test` | `note`, `dry_run` | Execute machine-verifiable `bash:verify` assertions under security ceilings. |
| `akatsuki_reconcile` | `akatsuki reconcile` | `dry_run` | Rebuild SQLite projection, auto-quote colon scalars, and sync vectors. |
| `akatsuki_list_notes` | `akatsuki list` | `domain` | List notes in the vault with metadata, status, and domain filtering. |

---

## ⚡ Vault Discovery Ladder

Akatsuki locates the target knowledge base automatically using a multi-tiered resolution ladder:

1. **CLI Flag**: `--vault /path/to/vault` (or `-V`)
2. **Environment Variable**: `AKATSUKI_VAULT=/path/to/vault`
3. **Upward Directory Walk**: Traverses upwards looking for `.akatsuki/` or `INDEX.md` + `AGENTS.md`.
4. **Well-Known Locations**: Checks `~/configs/knowledge-base/akatsuki`, `~/.akatsuki`, `~/akatsuki`.
5. **Fallback**: Current working directory.

---

## 🧪 Testing

Akatsuki is validated against an exhaustive regression and integration test suite:

```bash
# Run complete test suite (unit, CLI, and regression tests)
cargo test

# Run tests with output
cargo test -- --nocapture
```

The test harness asserts:
- Path traversal underflow containment and internal `.akatsuki/` mutation isolation.
- Read-only SQLite query validation and mutating CTE / SQL injection prevention.
- Security gate blocking destructive invariant commands (`rm`, `dd`, `mkfs`, fork bombs).
- Invariant timeout handling and asynchronous output buffer draining (capped at 2 MB).
- Wikilink code fence stripping and heading extraction with emoji/punctuation normalization.
- Token sequence clamping (512 tokens) to protect BERT positional embeddings.

---

## 📜 License

MIT License. Copyright (c) 2026 Yusuf Akçakaya.
