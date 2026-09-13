# 🌅 Akatsuki (暁)

> **Universal CLI and Model Context Protocol (MCP) Gateway for Living System Memory and Architectural Contracts for Autonomous AI Agents.**

[![CI](https://github.com/fusuyfusuy/akatsuki/actions/workflows/ci.yml/badge.svg)](https://github.com/fusuyfusuy/akatsuki/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Python: 3.10+](https://img.shields.io/badge/python-3.10+-blue.svg)](https://www.python.org/)
[![Zero Dependencies](https://img.shields.io/badge/dependencies-0%20(stdlib%20only)-success.svg)](https://docs.python.org/3/library/)

---

## 🏛️ What is Akatsuki?

As AI coding swarms and autonomous agents (`Antigravity`, `Claude Code`, `pi`, `OpenCode`, `Cursor`) write and refactor code, traditional static documentation decays. Infrastructure configurations drift, network ports clash, and architectural boundaries get silently breached.

**Akatsuki** solves this by providing a **Living Systems Memory Substrate**:
- **Machine-Verifiable Contracts**: Inspect living system contracts, service boundaries, and cluster topology before mutating code.
- **Living Invariant Testing (`bash:verify`)**: Test system health against machine-executable verification assertions embedded directly within Markdown notes.
- **Fast Okapi BM25 Search**: Zero-daemon, in-process SQLite full-text search with morphological expansion and excerpt ranking.
- **Native Dual-Interface (CLI + MCP)**: Operate seamlessly from the human terminal (`akatsuki <command>`) or equip autonomous agents directly through stdio Model Context Protocol (`akatsuki mcp`).
- **Zero Runtime Dependencies**: Written entirely in Python standard library (`sqlite3`, `pathlib`, `argparse`, `fcntl`, `json`). No heavy dependencies, no build friction.

---

## 🚀 Quick Start

### 1. Installation

Install via pip or uv:

```bash
# Via pip
pip install git+https://github.com/fusuyfusuy/akatsuki.git

# Or via uv tool
uv tool install git+https://github.com/fusuyfusuy/akatsuki.git
```

### 2. Bootstrap a Living Memory Vault

```bash
# Initialize a fresh living memory vault in the current directory or specified path
akatsuki init ./knowledge-base

cd ./knowledge-base
```

This creates the standard Akatsuki vault structure:
```
knowledge-base/
├── .akatsuki/          # Local SQLite BM25 search index and locks
├── .gitignore          # Pre-configured multi-machine ignores
├── AGENTS.md           # Master architectural protocol and invariants
├── INDEX.md            # Auto-maintained catalog and domain index
├── 01-Daily/           # Daily activity ledgers & worklogs
├── 20-Projects/        # Active software project architectures
├── 40-Systems/         # Host specifications, topologies & ADRs
└── _templates/         # Note, system, and daily templates
```

### 3. Basic CLI Commands

```bash
# Search notes using Okapi BM25 ranking
akatsuki search "docker swarm routing"

# Read a note or specific section with token budget packing
akatsuki read "Cluster-Topology" --section "Private Network Routing"

# Calculate blast radius for a service before making changes
akatsuki blast bountools

# Run living invariant checks across system documentation
akatsuki test

# Record a telegraphic caveman ledger entry
akatsuki log -p "web" -s "migrate edge certs -> let encrypt automated; exit 0"

# Surgically update frontmatter without corrupting note bodies
akatsuki set "20-Projects/filament" -k status -v "live"
```

---

## 🤖 MCP Server Setup (for AI Agents)

Akatsuki runs as a native stdio Model Context Protocol (MCP) server exposing 17 specialized architectural tools:

```bash
akatsuki mcp
```

### Claude Desktop Configuration (`claude_desktop_config.json`)
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

### Cursor Configuration (`.cursor/mcp.json`)
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

### Antigravity / OpenCode / pi Configuration
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

## 🛠️ MCP Tools Reference

| MCP Tool | Description |
| :--- | :--- |
| `akatsuki_search` | Search notes using Okapi BM25 ranking, stemming, and contextual snippets. |
| `akatsuki_read` | Read full notes or extract individual headings with token budget packing. |
| `akatsuki_contract` | Extract structured machine boundary contracts (APIs, ports, schemas) from notes. |
| `akatsuki_get` | $O(1)$ exact property getter across frontmatter keypaths. |
| `akatsuki_query` | Run read-only SQL queries directly against the internal SQLite FTS index. |
| `akatsuki_blast` | Compute upstream callers and downstream dependencies for architectural blast radius. |
| `akatsuki_test` | Execute machine-verifiable `bash:verify` assertion blocks. |
| `akatsuki_set` | Surgically update YAML frontmatter keys with JSON/primitive values. |
| `akatsuki_lint` | Validate vault notes against strict schema contracts. |
| `akatsuki_append_section` | Append markdown content under specific headings. |
| `akatsuki_services` | Read active service matrices, container prefixes, and port allocations. |
| `akatsuki_projects` | List all tracked software repositories and production deployments. |
| `akatsuki_daily` | Read today's or specified daily horizon and task list. |
| `akatsuki_record_log` | Append telegraphic ledger entries to today's active worklog. |
| `akatsuki_verify` | Verify internal wikilink graph integrity. |

---

## ⚡ Vault Discovery Ladder

Akatsuki locates the target knowledge base automatically using a multi-tiered resolution ladder:

1. **CLI Flag**: `--vault /path/to/vault` (or `-V`)
2. **Environment Variable**: `AKATSUKI_VAULT=/path/to/vault`
3. **Upward Directory Walk**: Traverses upwards from the current directory looking for `.akatsuki/` or `INDEX.md` + `AGENTS.md`.
4. **Well-Known Locations**: Checks `~/configs/knowledge-base/akatsuki`, `~/.akatsuki`, `~/akatsuki`.
5. **Fallback**: Current working directory.

---

## 🧪 Testing

Akatsuki comes with a comprehensive standard test suite with zero test dependencies:

```bash
# Run tests using Python standard library unittest
python3 -m unittest discover tests -v
```

---

## 📜 License

MIT License. Copyright (c) 2026 Yusuf Akçakaya.
