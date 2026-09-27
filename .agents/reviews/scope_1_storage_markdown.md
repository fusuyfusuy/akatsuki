---
scope: "Scope 1: Storage & Markdown Engine"
score: 8.8
status: "MINOR"
critical_findings: 0
invariant_breaches: []
---

# Scope 1 Audit: Storage & Markdown Engine

## 1. Executive Summary
- **Health Score**: 8.8 / 10.0 (MINOR).
- **Target Files**: `src/storage/mod.rs`, `src/constants.rs`, `tests/regression_tests.rs`.
- **Verdict**: The storage substrate and markdown engine are structurally sound and exhibit high fidelity to documented invariants. Critical vulnerabilities previously identified in v0.2.0 (relative path traversal underflow, code block comment heading collisions, and substring shadowing) have been resolved and verified by regression tests. Residual defects are bounded to internal hidden file whitelisting in `contained_path`, permission stripping in `write_atomic`, lack of lock acquisition timeouts, subdirectory omission in `is_raw_path`, and allocation pressure in heading normalization.

---

## 2. Invariant & Regression Verification
- **Code Fence Isolation** (`memory.md#L25`, `src/storage/mod.rs:492-506`): `locate_section` maintains fence state across both ```` ``` ```` and `~~~`, preventing embedded comments from prematurely terminating sections. Regression test `section_extraction_ignores_comments_inside_code_fences` passes.
- **Two-Pass Heading Resolution** (`src/storage/mod.rs:487-528`): Exact matches take precedence over partial substring matches, resolving earlier substring shadowing. Regression test `locate_section_prefers_exact_match_over_earlier_substring` passes.
- **Relative Path Underflow** (`src/storage/mod.rs:197-209, 245-265`): `contained_path` resolves relative vault paths against `current_dir()`, and `normalize_path` preserves `ParentDir` underflow components instead of dropping them. Regression test `contained_path_rejects_relative_underflow` passes.
- **Frontmatter Strictness** (`memory.md#L18`, `src/storage/mod.rs:273-315`): `parse_frontmatter` rejects unclosed fences, invalid YAML syntax, and non-mapping roots via `anyhow::Result`, preventing corruption on mutation. Regression test `set_property_refuses_unparseable_frontmatter` passes.

---

## 3. Dimensional Findings

### A. Correctness
1. **`is_raw_path` Fails on Subdirectories** (`src/storage/mod.rs:39-43`):
   Checks `lower == "dockerfile" || lower == "caddyfile" || lower == "makefile"`. If a file resides in a subdirectory (e.g., `20-Projects/app/Dockerfile`), `lower` is `"20-projects/app/dockerfile"`, evaluating to `false`. Without explicit `--raw`, `mutations::write_note` incorrectly appends `.md`.
2. **Vault Discovery Anchor Case Mismatch** (`src/storage/mod.rs:124` vs `src/constants.rs:28`):
   Upward walk checks `INDEX.md` and `AGENTS.md` (uppercase only). `ROOT_ANCHORS` in `constants.rs` defines `["index.md", "operator.md", "agents.md", "readme.md"]` (lowercase). Vaults with lowercase anchors on case-sensitive filesystems fail detection.
3. **`shellexpand` Omits Tilde Expansion** (`src/storage/mod.rs:150-190`):
   `shellexpand` only processes `$VAR` and `${VAR}`. Paths defined as `AKATSUKI_VAULT="~/vault"` in `~/.config/knowledge-base/env` or environment variables fail `pb.is_dir()` (`src/storage/mod.rs:80,101`).
4. **Token Budget Bypass on Malformed Fence** (`src/storage/mod.rs:640-655`):
   `apply_token_budget` enables `in_frontmatter = true` if `text.starts_with("---")`. If the opening fence is never closed, `fences == 2` is never reached, leaving `in_frontmatter = true` throughout and disabling token budgeting entirely.
5. **CRLF Retention in Frontmatter Separation** (`src/storage/mod.rs:312, 625`):
   `body.trim_start_matches('\n')` leaves leading carriage returns (`\r`) intact on CRLF documents.

### B. Robustness
1. **Unbounded Blocking Lock & Re-entrancy Risk** (`src/storage/mod.rs:16-34`, `memory.md#L19`):
   `VaultLock::acquire` executes `file.lock_exclusive()` without a timeout or retry ceiling. If an external process wedges, all operations stall indefinitely. Intra-process re-entrancy currently relies on naming conventions (`append_work_log_inner`) rather than thread-local/mutex re-entrancy tracking.
2. **Temp File Orphanage on Write Failure** (`src/storage/mod.rs:343`):
   In `write_atomic`, `fs::remove_file(&tmp_path)` is only called if `fs::rename` fails (L345). If `fs::write(&tmp_path, content)?` fails midway (e.g. disk quota), the temp file is not unlinked.
3. **Symlink Canonicalization vs Lexical Root** (`src/storage/mod.rs:231-240`):
   `contained_path` compares fully canonicalized symlink paths (`fs::canonicalize(&resolved)`) against `norm_vault` (which is only lexically normalized). If the vault path itself traverses a symlink (e.g., `/var` -> `/private/var`), valid in-vault symlinks can be rejected.

### C. Performance
1. **Allocation Churn in `normalize_heading`** (`src/storage/mod.rs:452-458`):
   Performs 4 distinct heap allocations per heading line (`collect::<String>()`, `to_lowercase()`, `Vec<&str>`, `join(" ")`). In `locate_section`, which executes a two-pass scan, this triggers repeated allocations for every heading in the file.
2. **Allocation Overhead in WalkDir Fallback** (`src/storage/mod.rs:434-435`):
   `resolve_note_file` executes `to_lowercase()` and `.replace('-', " ")` on every `.md` file stem in the vault when SQLite cache lookup misses.

### D. Security
1. **Internal State Mutation via `.akatsuki` Whitelist** (`src/storage/mod.rs:222`):
   `contained_path` explicitly exempts `.akatsuki` and `.akatsuki.lock` from hidden directory rejection. Public mutation entry points (`mutations::write_note`, `mutations::set_property`) that rely solely on `contained_path` can overwrite `.akatsuki/cache.db` or `.akatsuki.lock`.
2. **File Permission Stripping in `write_atomic`** (`src/storage/mod.rs:341-350`):
   `write_atomic` creates temporary replacement files with default umask (0644). It does not copy permissions from the existing target file before renaming, stripping executable bits (0755) from scripts in `60-Scripts` or exposing restrictive permissions (0600).
3. **Flock FD Lifecycle** (`src/storage/mod.rs:11-35`):
   Clean RAII implementation. The `File` descriptor is owned by `VaultLock`, ensuring deterministic `close()` and advisory lock release upon `Drop`. No descriptor leaks detected.

---

## 4. Prioritized Actionable Remediations

| Priority | Issue | Location | Remediation |
| :--- | :--- | :--- | :--- |
| **P1** | `.akatsuki` escape in mutations | `src/storage/mod.rs:222` | Reject `.akatsuki` / `.akatsuki.lock` in `contained_path` or restrict them to internal engine operations. |
| **P1** | File permission loss | `src/storage/mod.rs:341-350` | In `write_atomic`, query existing file permissions via `metadata()` and apply them to `tmp_path` before rename. |
| **P2** | `is_raw_path` subdirectory omission | `src/storage/mod.rs:39-43` | Check `Path::new(rel_path).file_name()` instead of full `lower` string equality for raw filenames. |
| **P2** | Unbounded lock acquisition | `src/storage/mod.rs:26` | Replace indefinite `lock_exclusive()` with non-blocking `try_lock_exclusive()` loop with configurable timeout. |
| **P2** | Temp file cleanup on write error | `src/storage/mod.rs:343` | Clean up `tmp_path` on `fs::write` error using an RAII cleanup guard. |
| **P3** | Anchor case sensitivity & tilde | `src/storage/mod.rs:124,150` | Support lowercase `index.md`/`agents.md` in upward walk; expand `~` using `dirs::home_dir()`. |
| **P3** | Heading allocation reduction | `src/storage/mod.rs:452-458` | Streamline `normalize_heading` to single allocation or parse headings into cached tuples during pass 1. |
