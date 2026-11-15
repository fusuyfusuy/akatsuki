# akatsuki

A small Rust CLI and MCP stdio server for keeping project, service, and daily notes in plain Markdown, backed by a rebuildable SQLite index.

Markdown files on disk remain the source of truth. `.akatsuki/cache.db` (SQLite WAL + FTS5, plus optional local Candle embeddings) is a derived cache that can be deleted and rebuilt at any time with `akatsuki reconcile`.

## Install

Lexical search only (SQLite FTS5 BM25):

```bash
cargo install --path .
```

With local vector search (`intfloat/multilingual-e5-small` via Candle, fused with BM25 using RRF):

```bash
cargo install --path . --features vectors
akatsuki setup-models   # downloads weights to ~/.cache/akatsuki/models/
```

## Vault layout

```text
vault/
├── .akatsuki/          # derived SQLite cache and lockfile (gitignored)
├── AGENTS.md           # global invariants
├── INDEX.md            # root map of contents
├── OPERATOR.md         # host/operator notes
├── 01-Daily/           # YYYY-MM-DD daily work logs
├── 20-Projects/        # project notes
├── 40-Systems/         # infrastructure notes & ADRs
└── _templates/         # optional templates (ignored by link checks)
```

## CLI usage

```bash
# Search (modes: hybrid, bm25, vector)
akatsuki search "wireguard routing" --with-graph
akatsuki search "wireguard routing" --mode bm25

# Read a note, table of contents, or a single section
akatsuki read Cluster-Topology --toc
akatsuki read Cluster-Topology --section "Routing" --budget 500

# Extract structured contract (ports, relations, invariants, verifications)
akatsuki contract 40-Systems/Cluster-Topology.md

# Dependency graph and blast radius
akatsuki map auth-service --depth 2 --direction both
akatsuki blast auth-service

# Update frontmatter, sections, and daily logs (under advisory flock + atomic rename)
akatsuki set 20-Projects/web --key status --value live
akatsuki append 20-Projects/web --heading "Notes" --content "- rotated TLS cert"
akatsuki replace 20-Projects/web --heading "Status" --content "Healthy."
akatsuki log --project web --summary "rotate edge cert -> renew SAN; exit 0"

# Lint frontmatter, check links/orphans, run bash:verify checks, or rebuild cache
akatsuki lint
akatsuki verify
akatsuki test --dry-run
akatsuki reconcile
```

Pass `--json` to any subcommand for structured output, or `--vault <dir>` to target a specific directory.

## MCP server

Run over stdio (accepts both newline-delimited JSON-RPC and `Content-Length` framing):

```json
{
  "mcpServers": {
    "akatsuki": {
      "command": "akatsuki",
      "args": ["mcp"],
      "env": {
        "AKATSUKI_VAULT": "/path/to/vault"
      }
    }
  }
}
```

### Tools (20)

| Tool | CLI | Arguments |
| :--- | :--- | :--- |
| `akatsuki_search` | `search` | `query`, `mode`, `domain`, `limit`, `with_graph` |
| `akatsuki_read` | `read` | `note` (`path`/`target`), `section`, `toc`, `budget` |
| `akatsuki_contract` | `contract` | `note` (`target`/`path`) |
| `akatsuki_blast` | `blast` | `target` (`note`/`path`) |
| `akatsuki_map` | `map` | `target` (`note`/`path`), `depth`, `direction` |
| `akatsuki_services` | `services` | *(none)* |
| `akatsuki_projects` | `projects` | *(none)* |
| `akatsuki_get` | `get` | `key` (`keypath`) |
| `akatsuki_query` | `query` | `sql` (read-only `SELECT`/`WITH`/`EXPLAIN`) |
| `akatsuki_list_notes` | `list` | `domain`, `limit` |
| `akatsuki_daily` | `daily` | `date` |
| `akatsuki_record_log` | `log` | `project`, `summary`, `device` |
| `akatsuki_write_note` | `write` | `path`, `content`, `overwrite`, `raw` |
| `akatsuki_set` | `set` | `note`, `key`, `value` |
| `akatsuki_append_section` | `append` | `note`, `heading`, `content` |
| `akatsuki_replace_section` | `replace` | `note`, `heading`, `content` |
| `akatsuki_lint` | `lint` | *(none)* |
| `akatsuki_verify` | `verify` | *(none)* |
| `akatsuki_test` | `test` | `note`, `dry_run` |
| `akatsuki_reconcile` | `reconcile` | `dry_run` |

### Resources

`resources/list`, `resources/templates/list`, and `resources/read` expose `akatsuki://services`, `akatsuki://projects`, `akatsuki://operator`, `akatsuki://daily`, `akatsuki://invariants`, `akatsuki://index`, and `akatsuki://{note}` (capped at 64 KB).

## Vault resolution & environment

Vault discovery checks, in order:
1. `--vault <dir>`
2. `$AKATSUKI_VAULT`
3. `AKATSUKI_VAULT=` in `~/.config/knowledge-base/env`
4. Upward directory walk from `cwd` (excluding `$HOME`) for `.akatsuki/`, `20-Projects/` + `40-Systems/`, or `INDEX.md` + `AGENTS.md`
5. Common local paths (`~/configs/knowledge-base/akatsuvault`, `~/.config/akatsuki`, `~/.akatsuki`, `~/akatsuki`)

Other environment variables:
- `AKATSUKI_HOST`: host name recorded in `updated_by` and `log` entries.
- `AKATSUKI_INVARIANT_TIMEOUT`: per-command timeout in seconds for `akatsuki test` (default `10`, total run budget `300s`).
- `AKATSUKI_INVARIANT_ENV`: `inherit` (default) or `clean` (`PATH`, `HOME`, `LANG` only).

**Trust model**: `akatsuki test` executes `bash:verify` code blocks from the vault in their own process group with stdin closed and output capped at 2 MiB. Only run `akatsuki test` on vaults you author or trust.

## Development

```bash
cargo test
cargo test --features vectors --lib
```

## License

MIT
