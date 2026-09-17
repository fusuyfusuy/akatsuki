# Project Memory

## Active Epics & Scale
- Scale: Modularized standard-library AI knowledge secretariat (4.8k LOC total across 13 modules, 43 unit tests).
- Architecture: Decoupled into `storage`, `index`, `vectors`, `search`, `graph`, `verify`, `mutations`, `mcp`, and `cli`, preserved by a backward-compatible `core.py` facade.

## KNOWN DEBT (open only — one line per item, delete when done)
# Deliberate gaps get ledger lines: - accepted <what> <- <why> -> <trigger>

## Domain Vocabulary & Gotchas
- Test Isolation: `get_external_embed_python` scans host paths unless `AKATSUKI_TESTING=1` or `AKATSUKI_DISABLE_HOST_EMBED=1` is set; always keep test environment isolated to prevent cold PyTorch subprocess latency.
- M2M Parity: All CLI subcommands strictly support `--json` output alongside human terminal formatting.
- Surgical Section Patching: `replace_section_in_note` atomically swaps markdown content under a specific heading without corrupting frontmatter or adjacent sections.
- Vault Override: `resolve_vault_path` checks both `akatsuki.storage.CURRENT_VAULT_OVERRIDE` and `akatsuki.core.CURRENT_VAULT_OVERRIDE` for multi-module and test runner compatibility.
