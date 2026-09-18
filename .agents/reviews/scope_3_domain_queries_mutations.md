---
scope: "queries-verification-mutations"
score: 7.6
status: "MODERATE"
critical_findings: 2
invariant_breaches:
  - "Unsandboxed shell execution in run_verification_tests (subprocess.run shell=True without path confinement)"
  - "Indentation destruction during reconcile_vault auto-quoting flattens nested YAML mappings"
---

# Deep Audit: Queries, Verification & Mutations

## 1. Executive Summary & Health Score
- **Overall Score**: 7.6 / 10 (`MODERATE`)
- **Primary Strengths**: Process-safe advisory locking (`VaultLock`) across mutations, atomic file writes via PID-tagged temp files and `os.replace`, robust frontmatter auto-healing, and Okapi BM25 FTS5 column weighting.
- **Key Vulnerabilities**: Arbitrary shell command execution via ```bash:verify``` blocks, YAML AST corruption in reconciliation, quadratic disk I/O in write loops (`verify_links` on note write), and non-atomic writes in `reconcile_vault`.

## 2. Findings Matrix

| Ref | Severity | File:Line | Category | Summary |
|---|---|---|---|---|
| F-01 | CRITICAL | [verify.py:68-74](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/verify.py#L68-L74) | Security | Unsandboxed `subprocess.run(cmd, shell=True)` executes untrusted vault commands with no `cwd` or path bounds. |
| F-02 | CRITICAL | [verify.py:345-362](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/verify.py#L345-L362) | Correctness | `reconcile_vault` strips YAML line indentation on auto-quote, flattening nested mapping structures into top-level keys. |
| F-03 | HIGH | [verify.py:322-416](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/verify.py#L322-L416) | Robustness | `reconcile_vault` performs in-place `write_text` without `VaultLock` or atomic temp files, risking corruption. |
| F-04 | HIGH | [mutations.py:270-274](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/mutations.py#L270-L274) | Performance | `write_note` triggers full vault parse `verify_links(vault)` ($O(N)$ file reads) on every single write operation. |
| F-05 | MEDIUM | [mutations.py:222-251](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/mutations.py#L222-L251) | Security | `contained_path` permits writes to `.git/` (e.g. `.git/hooks/pre-commit` + `chmod 0o755`), enabling hook execution. |
| F-06 | MEDIUM | [verify.py:281](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/verify.py#L281) | Security | Path prefix flaw: `str(resolved).startswith(str(vault))` allows adjacent folders like `akatsuki_evil` to pass check. |
| F-07 | MEDIUM | [search.py:257-288](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/search.py#L257-L288) | Correctness | Score scale disparity: hybrid fallback returns raw BM25 (5-10) vs RRF reciprocal scores (0.01-0.03). |
| F-08 | MEDIUM | [mutations.py:119-120](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/mutations.py#L119-L120) | Correctness | `append_section_to_note` forces `- ` prefix onto paragraphs, tables, code blocks, and blockquotes. |
| F-09 | MEDIUM | [mutations.py:292-299](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/mutations.py#L292-L299) | Correctness | `set_note_property` replaces existing list values with empty dicts when indexing attempts occur (`tags.0`). |
| F-10 | LOW | [search.py:108-116](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/search.py#L108-L116) | Correctness | `get_keypath` for `services` discards `parts[3:]`, failing to traverse nested service attributes. |
| F-11 | LOW | [verify.py:239-242](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/verify.py#L239-L242) | Robustness | `stems = {f.stem.lower(): f}` causes non-deterministic stem shadowing when files share names across folders. |
| F-12 | LOW | [mutations.py:58-62](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/mutations.py#L58-L62) | Robustness | `append_work_log` treats bash `# comment` inside fences as H1 headings, splitting code blocks. |
| F-13 | LOW | [mutations.py:326-352](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/mutations.py#L326-L352) | Performance | `list_notes_in_vault` reads every `.md` file from disk instead of querying indexed SQLite `entities` table. |
| F-14 | LOW | [search.py:293-317](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/search.py#L293-L317) | Performance | `with_graph` runs 3 separate SQL queries per result row in a loop (N+1 query bottleneck). |

## 3. Dimensional Deep Dive

### 3.1 Correctness
- **RRF Hybrid Search**: `cand_limit = max(limit * 2, 20)` with `k=60.0` works mathematically, but single-sided hits retain raw BM25 scores (`score > 1.0`), whereas fused hits are `0.015 - 0.033`. Callers relying on thresholding receive mismatched scales.
- **Section Replacement**: `replace_markdown_section` accurately computes 1-indexed boundaries and protects subsequent headings, but if replacement text starts with `#` (e.g. Markdown header or Bash comment), it discards the target heading line.
- **Keypath Resolution**: Discrepancy between `services` (max 1 property deep) and `entities` (arbitrary dictionary traversal).

### 3.2 Robustness
- **Reconciliation Concurrency**: `reconcile_vault` bypasses `VaultLock` and writes directly via `Path.write_text`. Under multi-agent concurrency, reconciliation can race with `append_work_log` or `write_note`.
- **Wikilink Resolution**: Stems are stored in a single-value dictionary `stems = {f.stem.lower(): f}`. Colliding stems (e.g. `20-Projects/api.md` vs `40-Systems/api.md`) shadow each other arbitrarily.

### 3.3 Performance
- **Write Amplification**: Every call to `write_note` invokes `verify_links(vault)`, forcing synchronous reads of every note in the vault. In a vault with 3,000 notes, writing 10 notes reads 30,000 files from disk.
- **Redundant Disk Scanning**: `list_notes_in_vault` re-reads all markdown files instead of reading from `entities`. `search_vault` runs incremental FTS sync (`glob` + `stat` on all files) on every single query call.

### 3.4 Security
- **Command Injection via bash:verify**: Markdown notes with ````bash:verify```` blocks execute arbitrary commands under `shell=True` without sandboxing or timeout overrides.
- **Hidden Vault Overwrites**: `contained_path` checks that target is within vault, but allows subpaths starting with `.` (e.g., `.git/hooks/pre-commit`), allowing arbitrary hook injection if scripts are marked executable.
- **Path Prefix Checking**: `str(resolved).startswith(str(vault))` should be replaced with `resolved.is_relative_to(vault)`.

## 4. Test Suite Evaluation
- **`tests/test_reconcile.py` (145 lines)**: Covers unquoted colon detection, graph closure unindexed detection, wikilink anchor parsing, and relations parsing. Defect: does not test nested YAML auto-quoting, multiple unindexed notes in same MOC, or concurrency.
- **`tests/test_device_tracking.py` (106 lines)**: Covers host resolution, work log formatting, frontmatter updating, and entity synchronization. Defect: does not test concurrent mutations, lock timeouts, or non-bullet work log content.

## 5. Prioritized Actionable Remediations
1. **Sanitize `bash:verify` Execution**: Require explicit `--allow-exec` flag or execute via restricted runner with explicit `cwd=vault` and sandboxed environment (`verify.py:68-74`).
2. **Preserve YAML Indentation**: Refactor auto-quoting in `reconcile_vault` to compute leading indentation (`len(line) - len(line.lstrip())`) before rewriting lines (`verify.py:345-362`).
3. **Lock & Atomize Reconcile**: Wrap `reconcile_vault` in `with VaultLock(vault):` and use atomic `.tmp.{pid}` + `os.replace` (`verify.py:362, 392`).
4. **Decouple `verify_links` from `write_note`**: Remove synchronous `verify_links` from `write_note` or guard behind an opt-in `verify=True` parameter (`mutations.py:270-274`).
5. **Enforce Dotfile Immunity in `contained_path`**: Reject write targets starting with `.` or containing `/.` (`storage.py:177-190`).
6. **Use `Path.is_relative_to`**: Replace string prefix checking with `resolved.is_relative_to(vault)` (`verify.py:281`).
7. **Leverage SQLite in `list_notes_in_vault`**: Query `entities` table directly instead of re-reading markdown files from disk (`mutations.py:326-352`).
