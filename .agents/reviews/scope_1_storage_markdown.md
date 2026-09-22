---
scope: "storage_markdown"
score: 7.8
status: "MODERATE"
critical_findings: 2
invariant_breaches:
  - "locate_section parses code block comments as headings, breaking section boundary invariant"
  - "contained_path allows traversal escape when vault path is relative (normalize_path underflow)"
---

# Scope Audit: Storage & Markdown Engine (Akatsuki v0.2.0)

## 1. Executive Summary
- **Health Score**: 7.8 / 10.0 (MODERATE).
- **Core Substrate**: Native POSIX storage layer handling atomic I/O, vault containment, YAML frontmatter parsing, section extraction, and file locking.
- **Verdict**: While frontmatter parsing strictness and atomic rename mechanics are solid, critical defects exist in markdown heading detection inside code blocks, relative path traversal in `contained_path`, and lock timeout/re-entrancy hygiene.

---

## 2. Invariant Breaches

1. **Section Boundary Corruption via Code Comments**:
   - **Contract**: [`memory.md#L17`](file:///home/devhax/projects/fusuyfusuy/akatsuki/.agents/memory.md#L17) asserts `storage::locate_section` is the single source of truth for heading lookups and section boundaries.
   - **Breach**: [`heading_at` in src/storage/mod.rs#L447-L454](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L447-L454) parses any line starting with `#` as a heading without requiring trailing whitespace (violating CommonMark ATX heading spec) and without checking for fenced code blocks (```` ``` ````). Any `# comment` inside a code block of level `<= matched_level` prematurely terminates the section, corrupting [`extract_section`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L490-L494), [`replace_section_in_note`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/mutations/mod.rs#L161-L179), and invariant indexing ([`src/index/mod.rs#L542`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/index/mod.rs#L542)).
2. **Containment Escape on Relative Vault Roots**:
   - **Contract**: [`contained_path` in src/storage/mod.rs#L192-L236](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L192-L236) must prevent directory traversal outside the vault.
   - **Breach**: [`normalize_path` in src/storage/mod.rs#L238-L250](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L238-L250) pops from an empty `components` vector on `ParentDir` (`..`). When `vault` is relative (e.g. `Path::new(".")`), `norm_vault` becomes `""`. Traversals like `../../etc/passwd` normalize to `etc/passwd`, where `"etc/passwd".starts_with("")` evaluates to `true`, returning an uncontained path outside the vault.

---

## 3. Dimensional Findings

### A. Correctness
- **`write_atomic` Durability & Leak on Error** ([`src/storage/mod.rs#L320-L338`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L320-L338)):
  Atomic `rename` ensures same-filesystem swap, but lacks `File::sync_all` before rename and parent directory fsync after rename, creating 0-byte risk on power loss. If `fs::write` fails at L328, the tempfile `tmp.<pid>.<ts>` is not unlinked.
- **`locate_section` Matching Order & Substring Collisions** ([`src/storage/mod.rs#L456-L488`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L456-L488)):
  Docstring claims "exact normalized match first, substring second", but L475 executes `if name == target || name.contains(&target)` in a single pass. An earlier heading containing `target` as a substring shadows a later exact match. Short headings (e.g. "log") falsely match substrings (e.g. "Dialog").
- **Frontmatter Parser Strictness & Truncation** ([`src/storage/mod.rs#L258-L300`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L258-L300), [`L588-L630`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L588-L630)):
  `parse_frontmatter` correctly validates YAML mappings and bails on non-mapping roots. However, `apply_token_budget` tracks frontmatter fences naively: if an opening `---` fence is never closed, `in_frontmatter` remains `true` indefinitely and truncation is bypassed.

### B. Robustness
- **`VaultLock` Unbounded Wait & No Re-entrancy Guard** ([`src/storage/mod.rs#L11-L35`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L11-L35)):
  `file.lock_exclusive()` blocks indefinitely via `flock(fd, LOCK_EX)`. If a stale process hangs, all callers stall without timeout. Furthermore, POSIX `flock` on a distinct fd in the same process deadlocks; `VaultLock` lacks process/thread-level re-entrancy tracking (e.g., thread-local recursion depth counter).
- **Symlink Canonicalization Discrepancy** ([`src/storage/mod.rs#L227-L232`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L227-L232)):
  `contained_path` compares canonicalized symlink targets (`fs::canonicalize`) against `norm_vault`. If `vault` itself is uncanonicalized or resides on a symlinked path (e.g. macOS `/var` -> `/private/var`), valid in-vault symlinks falsely fail `starts_with(&norm_vault)`.

### C. Performance
- **Allocation Churn in `normalize_heading`** ([`src/storage/mod.rs#L436-L444`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L436-L444)):
  Performs 4 distinct heap allocations per call (`collect::<String>()`, `to_lowercase()`, `Vec<_>`, `join(" ")`). In `locate_section`, this runs on every heading line across notes, inducing high GC-free allocator pressure.
- **Filesystem Scan Fallback Churn** ([`src/storage/mod.rs#L418-L425`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L418-L425)):
  `resolve_note_file` executes `to_lowercase()` and `.replace('-', " ")` on every single file stem during walkdir fallback.

### D. Security
- **Internal State Escape via `.akatsuki` Whitelist** ([`src/storage/mod.rs#L215-L217`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/storage/mod.rs#L215-L217)):
  `contained_path` explicitly exempts `.akatsuki` and `.akatsuki.lock` from hidden directory rejection. An unprivileged caller calling `write_note(vault, ".akatsuki/cache.db", ...)` can overwrite or corrupt the SQLite database.

---

## 4. Prioritized Actionable Remediations

| Priority | Component | Remediation |
| :--- | :--- | :--- |
| **P0 (Critical)** | `heading_at` & `locate_section` | Enforce CommonMark ATX rule: require space/tab after `#` (`c == ' ' \|\| c == '\t'`). Track code block state (`in_code_block ^= line.starts_with("```")`) to ignore code comments. |
| **P0 (Critical)** | `contained_path` | Canonicalize `vault` root immediately; if `canonicalize(vault)` fails, reject relative traversal by ensuring `normalize_path` preserves leading `..` underflow or fails containment. |
| **P1 (High)** | `locate_section` | Implement two-pass search (pass 1: exact normalized match; pass 2: word-boundary substring match). |
| **P1 (High)** | `VaultLock` | Add acquisition timeout (`try_lock_exclusive` with retry ceiling) and a thread-local re-entrancy counter to prevent self-deadlock. |
| **P2 (Medium)** | `contained_path` | Remove `.akatsuki` and `.akatsuki.lock` exemptions from public note mutation endpoints. |
| **P2 (Medium)** | `write_atomic` | Add `sync_all` before rename, and clean up tempfile on write error via guard/defer. |
| **P3 (Low)** | `normalize_heading` | Use single-pass zero-allocation iterator comparison or single buffered normalization. |
