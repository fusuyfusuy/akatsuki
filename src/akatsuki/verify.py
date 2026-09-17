"""Verification assertions, schema linting, link checking, and vault reconciliation."""

import ast
import json
import re
import subprocess
from pathlib import Path

from akatsuki.constants import DOMAIN_MOCS, HAVE_PYYAML
from akatsuki.index import fts_db_context, sync_fts_index
from akatsuki.storage import validate_frontmatter_yaml

if HAVE_PYYAML:
    import yaml


def run_verification_tests(
    vault: Path,
    note_filter: str | None = None,
    as_json: bool = False,
    dry_run: bool = False,
) -> tuple[str, bool]:
    """Execute machine-verifiable bash:verify assertion blocks in notes."""
    with fts_db_context(vault) as con:
        sync_fts_index(vault, con)

        if note_filter:
            stem = Path(note_filter).stem
            cur = con.execute(
                "SELECT source_rel, command FROM verifications WHERE source_rel LIKE ? OR source_rel LIKE ?",
                (f"%{stem}.md", f"%{stem}%"),
            )
        else:
            cur = con.execute("SELECT source_rel, command FROM verifications")

        rows = cur.fetchall()

    if not rows:
        msg = "No machine verification blocks (```bash:verify) found in target."
        if as_json:
            return json.dumps({"total": 0, "passed": 0, "failed": 0, "results": [], "message": msg}, indent=2), False
        return msg, False

    results = []
    total = len(rows)
    passed = 0
    failed = 0

    for r in rows:
        src = r["source_rel"]
        cmd = r["command"].strip()

        if dry_run:
            results.append(
                {
                    "source": src,
                    "command": cmd,
                    "exit_code": 0,
                    "passed": True,
                    "stdout": "[DRY RUN - Command not executed]",
                    "stderr": "",
                }
            )
            passed += 1
            continue

        try:
            res = subprocess.run(
                cmd,
                shell=True,
                capture_output=True,
                text=True,
                timeout=5,
            )
            ok = res.returncode == 0
            if ok:
                passed += 1
            else:
                failed += 1
            results.append(
                {
                    "source": src,
                    "command": cmd,
                    "exit_code": res.returncode,
                    "passed": ok,
                    "stdout": res.stdout.strip(),
                    "stderr": res.stderr.strip(),
                }
            )
        except subprocess.TimeoutExpired:
            failed += 1
            results.append(
                {
                    "source": src,
                    "command": cmd,
                    "exit_code": 124,
                    "passed": False,
                    "stdout": "",
                    "stderr": "Command timed out after 5 seconds",
                }
            )
        except Exception as e:
            failed += 1
            results.append(
                {
                    "source": src,
                    "command": cmd,
                    "exit_code": 1,
                    "passed": False,
                    "stdout": "",
                    "stderr": str(e),
                }
            )

    if as_json:
        payload = {
            "total": total,
            "passed": passed,
            "failed": failed,
            "dry_run": dry_run,
            "results": results,
        }
        return json.dumps(payload, indent=2, default=str), (failed > 0)

    mode_str = " (DRY RUN)" if dry_run else ""
    out = [f"Ran {total} verification assertion(s){mode_str}: {passed} PASSED, {failed} FAILED\n"]
    for res in results:
        status_icon = "✅" if res["passed"] else "❌"
        out.append(f"{status_icon} [{res['source']}] exit {res['exit_code']}: `{res['command']}`")
        if not res["passed"]:
            if res["stderr"]:
                out.append(f"    stderr: {res['stderr']}")
            if res["stdout"]:
                out.append(f"    stdout: {res['stdout']}")

    return "\n".join(out), (failed > 0)


def lint_vault(vault: Path, as_json: bool = False) -> tuple[str, bool]:
    """Validate that all notes comply with strict machine schemas."""
    with fts_db_context(vault) as con:
        sync_fts_index(vault, con)

        cur = con.execute("SELECT rel_path, stem, type, metadata_json FROM entities")
        rows = cur.fetchall()
    errors = []

    required_by_type = {
        "project": ["title", "date", "type", "tags", "summary", "status"],
        "system": ["title", "date", "type", "tags", "summary"],
        "daily": ["title", "date", "type", "tags", "summary"],
        "agent": ["title", "date", "type", "tags", "summary"],
        "config": ["title", "date", "type", "tags", "summary"],
        "script": ["title", "date", "type", "tags", "summary"],
    }

    for r in rows:
        rel = r["rel_path"]
        note_type = r["type"]
        meta = json.loads(r["metadata_json"])
        needed = required_by_type.get(note_type, ["title", "date", "type", "summary"])
        missing = [f for f in needed if not meta.get(f)]
        if missing:
            errors.append(f"'{rel}' [{note_type}]: Missing required field(s): {', '.join(missing)}")

        # Strict YAML validation on markdown frontmatter
        note_path = vault / rel
        if note_path.exists():
            try:
                raw_text = note_path.read_text(encoding="utf-8")
                if raw_text.startswith("---"):
                    parts = raw_text.split("---", 2)
                    if len(parts) >= 3:
                        y_errs = validate_frontmatter_yaml(parts[1])
                        for ye in y_errs:
                            errors.append(f"'{rel}': Strict YAML frontmatter syntax error ({ye})")
            except Exception as e:
                errors.append(f"'{rel}': File read error during lint: {e}")

    # Validate syntax for non-markdown configs and scripts
    raw_files = []
    for d in ("50-Configs", "60-Scripts"):
        dp = vault / d
        if dp.exists():
            for f in dp.glob("**/*"):
                if f.is_file() and not f.name.startswith("."):
                    raw_files.append(f)

    for f in raw_files:
        rel = str(f.relative_to(vault))
        if f.suffix in (".yml", ".yaml"):
            try:
                if HAVE_PYYAML:
                    with open(f, encoding="utf-8") as yf:
                        yaml.safe_load(yf)
            except Exception as e:
                errors.append(f"'{rel}': YAML syntax error: {e}")
        elif f.suffix == ".json":
            try:
                with open(f, encoding="utf-8") as jf:
                    json.load(jf)
            except Exception as e:
                errors.append(f"'{rel}': JSON syntax error: {e}")
        elif f.suffix == ".py":
            try:
                with open(f, encoding="utf-8") as pf:
                    ast.parse(pf.read(), filename=str(f))
            except Exception as e:
                errors.append(f"'{rel}': Python syntax error: {e}")
        elif f.suffix == ".sh":
            try:
                res = subprocess.run(["bash", "-n", str(f)], capture_output=True, text=True)
                if res.returncode != 0:
                    errors.append(f"'{rel}': Bash syntax error: {res.stderr.strip()}")
            except Exception:
                pass

    if as_json:
        payload = {
            "passed": len(errors) == 0,
            "total_notes": len(rows),
            "errors": errors,
        }
        return json.dumps(payload, indent=2, default=str), (len(errors) > 0)

    if errors:
        out = [f"FAILED: {len(errors)} lint violation(s) found in akatsuki:"]
        for e in errors[:15]:
            out.append(f"  - {e}")
        return "\n".join(out), True
    return f"PASSED: All {len(rows)} notes conform to schema specifications.", False


def verify_links(vault: Path) -> tuple[bool, list[tuple[str, str]]]:
    """Verify all wikilinks, markdown links, and note connectivity across the vault."""
    files = list(vault.glob("**/*.md"))
    valid_files = [f for f in files if not str(f.relative_to(vault)).startswith(("_templates", "."))]

    stems = {f.stem.lower(): f for f in valid_files}
    rels = {str(f.relative_to(vault).with_suffix("")).lower(): f for f in valid_files}

    inbound_links = {str(f.relative_to(vault)): set() for f in valid_files}
    issues: list[tuple[str, str]] = []

    fence_pattern = r"```[\s\S]*?```"
    inline_pattern = r"`[^`\n]+`"

    for f in valid_files:
        src_rel = str(f.relative_to(vault))
        try:
            text = f.read_text(encoding="utf-8")
        except Exception as e:
            issues.append((src_rel, f"Unreadable file: {e}"))
            continue

        # Strip code blocks to prevent false positive matching on doc examples/audits
        clean_text = re.sub(fence_pattern, "", text)
        clean_text = re.sub(inline_pattern, "", clean_text)

        # 1. Wikilinks [[target|display]]
        for m in re.findall(r"(?<!\\)\[\[([^\]\|]+)(?:\|[^\]]+)?\]\]", clean_text):
            tgt = m.strip()
            target_no_anchor = tgt.split("#")[0].strip()
            if not target_no_anchor:
                continue
            tgt_clean = target_no_anchor[:-3] if target_no_anchor.endswith(".md") else target_no_anchor
            target_key = tgt_clean.lower()
            cand = rels.get(target_key) or stems.get(Path(tgt_clean).stem.lower())
            if cand is not None:
                inbound_links[str(cand.relative_to(vault))].add(src_rel)
            else:
                issues.append((src_rel, f"Broken wikilink: [[{tgt}]]"))

        # 2. Markdown links [text](path)
        for m in re.findall(r"\[(?:[^\]]*)\]\(([^)#\s]+)(?:#[^)]*)?\)", clean_text):
            tgt = m.strip()
            if tgt.startswith(("http://", "https://", "file://", "mailto:", "#")):
                continue
            resolved = (f.parent / tgt).resolve()
            if resolved.exists():
                if resolved.is_file() and str(resolved).startswith(str(vault)):
                    cand_rel = str(resolved.relative_to(vault))
                    if cand_rel in inbound_links:
                        inbound_links[cand_rel].add(src_rel)
            else:
                issues.append((src_rel, f"Broken markdown link: [{tgt}]"))

    # 3. Orphan Note Accounting
    root_anchors = {"INDEX.md", "OPERATOR.md", "AGENTS.md", "README.md"}
    for rel, inbounds in inbound_links.items():
        if rel in root_anchors or rel.startswith("01-Daily/"):
            continue
        if rel.startswith("50-Configs/") and not rel.endswith("Configs-MOC.md"):
            continue
        if rel.startswith("60-Scripts/") and not rel.endswith("Scripts-MOC.md"):
            continue
        if not inbounds:
            issues.append((rel, "Orphan note: No inbound links from MOC or index"))

    # 4. Graph Closure: Every note in primary domains must be indexed in parent MOC or INDEX.md
    for rel, inbounds in inbound_links.items():
        if (
            rel in root_anchors
            or rel.startswith("01-Daily/")
            or rel in DOMAIN_MOCS.values()
            or rel == "40-Systems/ADRs/ADRs-MOC.md"
        ):
            continue
        domain = rel.split("/")[0] if "/" in rel else ""
        parent_moc = DOMAIN_MOCS.get(domain)
        if not parent_moc:
            continue
        indexers = {"INDEX.md", parent_moc}
        if rel.startswith("40-Systems/ADRs/"):
            indexers.add("40-Systems/ADRs/ADRs-MOC.md")
        if not inbounds.intersection(indexers):
            issues.append((rel, f"Unindexed note: Not linked in parent MOC ({parent_moc}) or INDEX.md"))

    return len(issues) == 0, issues


def reconcile_vault(vault: Path, dry_run: bool = False, with_vectors: bool = False) -> tuple[str, bool]:
    """Scan vault, auto-quote strict YAML fields, and auto-append unindexed notes to parent MOCs."""
    actions = []

    files = list(vault.glob("**/*.md"))
    valid_files = [f for f in files if not str(f.relative_to(vault)).startswith(("_templates", "."))]

    # 1. Check strict YAML frontmatter quoting
    for f in valid_files:
        rel = str(f.relative_to(vault))
        try:
            text = f.read_text(encoding="utf-8")
        except Exception:
            continue
        if not text.startswith("---"):
            continue
        parts = text.split("---", 2)
        if len(parts) < 3:
            continue
        fm_raw = parts[1]
        body = parts[2]
        needs_write = False
        new_fm_lines = []
        for line in fm_raw.splitlines():
            stripped = line.strip()
            if ":" in stripped and not stripped.startswith(("-", "#")):
                k, v = stripped.split(":", 1)
                val = v.strip()
                # Check for unquoted scalar with colon-space
                if ": " in val and not (
                    (val.startswith('"') and val.endswith('"')) or (val.startswith("'") and val.endswith("'"))
                ):
                    escaped_val = val.replace('"', '\\"')
                    new_fm_lines.append(f'{k}: "{escaped_val}"')
                    needs_write = True
                    actions.append(f"Auto-quoted frontmatter field '{k}' in {rel}")
                    continue
            new_fm_lines.append(line)
        if needs_write and not dry_run:
            new_content = "---\n" + "\n".join(new_fm_lines) + "\n---" + body
            f.write_text(new_content, encoding="utf-8")

    # 2. Reconcile unindexed notes to domain MOCs
    _ok, issues = verify_links(vault)
    unindexed = [item for item in issues if item[1].startswith("Unindexed note")]

    for rel, _issue in unindexed:
        domain = rel.split("/")[0] if "/" in rel else ""
        parent_moc_rel = DOMAIN_MOCS.get(domain)
        if not parent_moc_rel:
            continue
        parent_moc = vault / parent_moc_rel
        if not parent_moc.exists():
            continue

        moc_text = parent_moc.read_text(encoding="utf-8")
        stem = Path(rel).stem
        target_link = f"[[{rel[:-3] if rel.endswith('.md') else rel}]]"
        stem_link = f"[[{stem}]]"

        if target_link in moc_text or stem_link in moc_text:
            continue

        entry_line = f"- {target_link}\n"
        actions.append(f"Appended unindexed note '{rel}' to {parent_moc_rel}")

        if not dry_run:
            if not moc_text.endswith("\n"):
                moc_text += "\n"
            parent_moc.write_text(moc_text + entry_line, encoding="utf-8")

    # 3. Synchronize FTS5 index
    if not dry_run:
        with fts_db_context(vault) as con:
            sync_fts_index(vault, con)

    # 4. Optional Vector index sync
    if with_vectors and not dry_run:
        try:
            from akatsuki.vectors import sync_vectors_index

            v_res = sync_vectors_index(vault)
            actions.append(f"Synchronized vector index: {v_res}")
        except Exception as e:
            actions.append(f"Vector sync deferred or skipped: {e}")

    mode_prefix = "[DRY-RUN] " if dry_run else ""
    if not actions:
        return f"{mode_prefix}Vault is fully reconciled (0 syntax errors, 0 unindexed notes).", False

    summary = [f"{mode_prefix}Vault reconciliation completed ({len(actions)} action(s) taken):"]
    for a in actions:
        summary.append(f"  - {a}")
    return "\n".join(summary), False
