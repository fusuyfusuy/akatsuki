---
scope: "storage-and-markdown"
score: 8.2
status: "MODERATE"
critical_findings: 0
invariant_breaches:
  - "src/akatsuki/storage.py:386 ensure_daily_note lacks contained_path boundary check on date_str"
  - "src/akatsuki/storage.py:197 parse_frontmatter splits on raw substring '---' rather than line-anchored '^---$'"
---

# Storage, Markdown & Parsing Engine Audit

## 1. Executive Summary
- **Health Score**: 8.2 / 10 (`MODERATE`)
- **Target Boundary**: `storage.py`, `markdown.py`, `constants.py`, and test harness coverage (`test_device_tracking.py`, `test_cli.py`).
- **Verdict**: Core storage abstractions and path confinement (`contained_path`) provide strong security defaults. However, two invariant breaches—raw substring frontmatter splitting on `---` and unconstrained path joining in `ensure_daily_note`—along with fallback YAML serializer defects and fence-state desynchronization in markdown parsing warrant prioritized remediation.

## 2. Review Dimensions

### Correctness
- **Frontmatter Delimiter Parsing** ([`storage.py:197`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L197)): `content.split("---", 2)` matches `---` anywhere in the line (e.g. em-dashes inside titles/summaries: `title: "Alpha --- Beta"`). This truncates frontmatter prematurely, leaves corrupt YAML, and injects partial frontmatter into the note body.
- **Fallback YAML Serializer** ([`storage.py:61-75`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L61-L75)): When `HAVE_PYYAML` is False, lists of dictionaries serialize using Python `str(dict)` (e.g., `"- \"{'a': 1}\""`), corrupting structured data. Furthermore, `_yaml_format_scalar` ([`storage.py:47-48`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L47-L48)) does not escape literal `\n` to `\n`, producing invalid multiline scalars.
- **Fallback Type Drift** ([`storage.py:268-270`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L268-L270)): When `HAVE_PYYAML` is False, null/empty scalars (`key:`) are coerced to empty lists `[]` instead of `None`, failing contract checks in `validate_note_content`.
- **Heading Extraction & Fences** ([`markdown.py:23-25`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/markdown.py#L23-L25)): Fences toggle on `line.strip().startswith("```")`. A 4-backtick outer fence enclosing a 3-backtick inner block toggles `in_code_fence` off early, parsing embedded `#` code comments as document headings.

### Robustness & Fault Tolerance
- **Null-Byte Injection** ([`storage.py:182-186`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L182-L186)): `contained_path` does not trap `ValueError` on embedded null bytes (`\0`), allowing unhandled runtime exceptions on hostile paths instead of returning `None`.
- **Process Locking Portability** ([`storage.py:360-376`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L360-L376)): `VaultLock` relies solely on `fcntl.flock`. On platforms where `HAVE_FCNTL` is False, locking becomes a silent no-op, removing all concurrency guarantees.
- **Fail-Fast in Library Code** ([`storage.py:173`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L173)): `get_vault` invokes `sys.exit(1)` when a vault is missing, terminating long-running parent processes (such as the MCP stdio server) instead of raising `FileNotFoundError`.

### Performance & Scalability
- **Unbounded Vault Globbing** ([`storage.py:333`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L333)): `resolve_note_file` executes `vault.glob("**/*.md")` across the entire vault without directory pruning (`.git`, `.venv`, `node_modules`), resulting in $O(N)$ filesystem latency on stem lookups.
- **Token Budget Packing** ([`markdown.py:158`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/markdown.py#L158)): Truncation threshold calculation `current_chars + len(line) > char_budget - 120` evaluates total text including frontmatter. Notes with frontmatter larger than `(budget * 4) - 120` discard all body lines immediately.

### Security
- **Path Traversal in Daily Provisioning** ([`storage.py:386`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L386)): `daily_file = daily_dir / f"{date_str}.md"` bypasses `contained_path`. Supplying a traversal string (e.g., `../../escaped`) writes files outside `01-Daily` and potentially escapes the vault root.
- **Atomic Write Inconsistency** ([`storage.py:412`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L412)): Unlike mutations in `mutations.py` which write to `.tmp.<pid>` before `os.replace`, `ensure_daily_note` calls direct `daily_file.write_text()`, risking partial writes on concurrent executions or abrupt termination.
- **YAML Deserialization Security**: Confirmed safe. `yaml.safe_load` is consistently utilized in `parse_frontmatter` ([`storage.py:206`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L206)) and `validate_frontmatter_yaml` ([`storage.py:280`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L280)).

## 3. Findings Matrix

| Ref | Location | Severity | Category | Description |
|---|---|---|---|---|
| F-01 | [`storage.py:386`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L386) | High | Security | Path traversal vulnerability in `ensure_daily_note` via unvalidated `date_str`. |
| F-02 | [`storage.py:197`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L197) | High | Correctness | `content.split("---", 2)` breaks on em-dashes `---` within frontmatter values. |
| F-03 | [`storage.py:412`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L412) | Medium | Robustness | Non-atomic file write in `ensure_daily_note` without `.tmp` + `os.replace`. |
| F-04 | [`storage.py:61-75`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L61-L75) | Medium | Correctness | Serializer fallback emits `str(dict)` for list of dicts; leaves `\n` unescaped. |
| F-05 | [`markdown.py:23-28`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/markdown.py#L23-L28) | Medium | Correctness | Code fence state machine desynchronizes on nested backticks / differing lengths. |
| F-06 | [`markdown.py:158`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/markdown.py#L158) | Medium | Robustness | Token budget arithmetic causes false-positive total truncation on rich frontmatter. |
| F-07 | [`storage.py:333`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L333) | Low | Performance | Unpruned `vault.glob("**/*.md")` incurs quadratic overhead on large vaults. |
| F-08 | [`storage.py:182-186`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L182-L186) | Low | Robustness | `contained_path` raises uncaught `ValueError` on null bytes in `rel_path`. |
| F-09 | [`storage.py:173`](file:///home/devhax/projects/fusuyfusuy/akatsuki/src/akatsuki/storage.py#L173) | Low | Robustness | `sys.exit(1)` in `get_vault()` library function terminates parent runtime. |

## 4. Test Suite Coverage Assessment
- **`tests/test_device_tracking.py`**: Tests hostname resolution, work log formatting, and SQLite indexing. Misses edge-case frontmatter parsing, lock contention, and path confinement on log writes.
- **`tests/test_cli.py`**: Validates basic happy-path `parse_frontmatter` (`title`, `tags: [a, b]`) and CLI arg parsing. Completely lacks negative test cases for traversal paths, malformed/complex frontmatter, nested code blocks, or fallback serializers.

## 5. Prioritized Actionable Remediations

- **P0 (Immediate)**:
  1. Patch `parse_frontmatter` to split on line-anchored regex `r"^---\s*$"` rather than raw substring `---`.
  2. Enforce `contained_path` in `ensure_daily_note` or validate `date_str` against regex `r"^\d{4}-\d{2}-\d{2}$"`.
- **P1 (Near-term)**:
  1. Convert `ensure_daily_note` write to atomic rename pattern (`.tmp.<pid>` -> `os.replace`).
  2. Implement CommonMark fence tracking in `extract_headings` (tracking fence length and fence character).
  3. Fix `_yaml_format_scalar` newline escaping and `dump_frontmatter` list-of-dict formatting.
  4. Recalibrate `apply_token_budget` threshold relative to body length rather than total text.
- **P2 (Hygiene)**:
  1. Add exception handling (`ValueError`, `OSError`) in `contained_path`.
  2. Prune ignore directories (`.git`, `node_modules`, `.akatsuki`) during `resolve_note_file` glob walks.
  3. Replace `sys.exit(1)` in `get_vault()` with `VaultNotFoundError(RuntimeError)`.
