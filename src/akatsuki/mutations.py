"""Vault mutation operations: write, set, append, replace, and activity logging."""

import datetime
import json
import os
import re
from pathlib import Path

from akatsuki.index import fts_db_context, sync_fts_index
from akatsuki.markdown import replace_markdown_section
from akatsuki.storage import (
    VaultLock,
    auto_heal_frontmatter,
    contained_path,
    dump_frontmatter,
    ensure_daily_note,
    get_machine_id,
    is_raw_path,
    parse_frontmatter,
    resolve_note_file,
    validate_note_content,
)
from akatsuki.verify import verify_links


def append_work_log(vault: Path, project: str, summary: str, device: str | None = None) -> str:
    """Append a timestamped work log entry to today's daily note under kernel lock."""
    now = datetime.datetime.now().astimezone()
    date_str = now.strftime("%Y-%m-%d")
    time_str = now.strftime("%H:%M")
    dev = device or get_machine_id()

    with VaultLock(vault):
        daily_note = ensure_daily_note(vault, date_str)
        content = daily_note.read_text(encoding="utf-8")

        prefix = f"[{project}]" if project else ""
        dev_tag = f"[{dev}]" if dev else ""
        if dev_tag and prefix:
            header = f"- **{time_str}** {dev_tag}: {prefix} {summary}"
        elif dev_tag:
            header = f"- **{time_str}** {dev_tag}: {summary}"
        elif prefix:
            header = f"- **{time_str}**: {prefix} {summary}"
        else:
            header = f"- **{time_str}**: {summary}"
        entry = header.strip()

        target_heading = "## 📝 Work Log & Session Notes"
        if target_heading in content:
            head, tail = content.split(target_heading, 1)
            entry_line = f"{entry}\n"

            if not tail.strip():
                tail = "\n" + entry_line
            else:
                lines = tail.splitlines(keepends=True)
                at = next(
                    (i for i, ln in enumerate(lines) if ln.startswith("#") and len(ln) - len(ln.lstrip("#")) <= 2),
                    len(lines),
                )
                body = lines[:at]
                while body and not body[-1].strip():
                    body.pop()
                if body and body[-1].strip() == "-":
                    body.pop()
                if body:
                    body.append("\n")
                body.append(entry_line)
                if at < len(lines):
                    body.append("\n")
                tail = "".join(body + lines[at:])

            new_content = head + target_heading + tail
        else:
            new_content = content.rstrip() + f"\n\n{target_heading}\n{entry}\n"

        tmp_file = daily_note.with_name(f".{daily_note.name}.tmp.{os.getpid()}")
        tmp_file.write_text(new_content, encoding="utf-8")
        os.replace(tmp_file, daily_note)

    return f"Recorded entry in {daily_note.name}: {entry}"


def append_section_to_note(vault: Path, rel_path: str, heading: str, content_to_append: str) -> tuple[str, bool]:
    """Safely append content under a specific heading inside an existing note."""
    clean_rel = rel_path.strip()
    if clean_rel.startswith("/") or clean_rel.startswith("~"):
        return ("Error: Absolute paths are not accepted; use a vault-relative path.", True)
    if not clean_rel.endswith(".md"):
        clean_rel += ".md"

    target_file = resolve_note_file(vault, clean_rel)
    if not target_file:
        target_file = contained_path(vault, clean_rel)
        if target_file is None:
            return "Error: Path traversal outside vault boundary is forbidden.", True

    with VaultLock(vault):
        if not target_file.exists():
            stem = target_file.stem
            body = f"# {stem}\n\n## {heading}\n{content_to_append.strip()}\n"
            healed_content = auto_heal_frontmatter(body, clean_rel)
            target_file.parent.mkdir(parents=True, exist_ok=True)
            tmp_file = target_file.with_name(f".{target_file.name}.tmp.{os.getpid()}")
            tmp_file.write_text(healed_content, encoding="utf-8")
            os.replace(tmp_file, target_file)
            return (f"Created '{clean_rel}' and appended section '{heading}'.", False)

        content = target_file.read_text(encoding="utf-8")
        fm, body = parse_frontmatter(content)
        fm["updated"] = datetime.datetime.now().astimezone().isoformat(timespec="seconds")
        fm["updated_by"] = get_machine_id()

        heading_pattern = re.compile(rf"^(#{{1,4}})\s+{re.escape(heading)}\s*$", re.MULTILINE)
        m = heading_pattern.search(body)

        to_add = content_to_append.strip()
        if not to_add.startswith("- ") and not to_add.startswith("#"):
            to_add = f"- {to_add}"

        if m:
            level = len(m.group(1))
            sec_start = m.end()
            next_heading = re.compile(rf"^#{{1,{level}}}\s+", re.MULTILINE)
            next_m = next_heading.search(body, sec_start)

            if next_m:
                insert_pos = next_m.start()
                new_body = (
                    body[:insert_pos].rstrip()
                    + "\n"
                    + to_add
                    + "\n\n"
                    + body[insert_pos:].lstrip()
                )
            else:
                new_body = body.rstrip() + "\n" + to_add + "\n"
        else:
            new_body = body.rstrip() + f"\n\n## {heading}\n{to_add}\n"

        final_content = dump_frontmatter(fm, new_body.lstrip())
        tmp_file = target_file.with_name(f".{target_file.name}.tmp.{os.getpid()}")
        tmp_file.write_text(final_content, encoding="utf-8")
        os.replace(tmp_file, target_file)

        try:
            with fts_db_context(vault) as db:
                sync_fts_index(vault, db)
        except Exception:
            pass

    if not clean_rel.startswith("01-Daily"):
        try:
            append_work_log(vault, project="akatsuki", summary=f"append {clean_rel} #{heading} -> exit 0")
        except Exception:
            pass

    return f"Successfully appended content under '{heading}' in '{clean_rel}'.", False


def replace_section_in_note(vault: Path, rel_path: str, heading: str, new_content: str) -> tuple[str, bool]:
    """Surgically replace the content of a specific heading in an existing note."""
    clean_rel = rel_path.strip()
    if clean_rel.startswith("/") or clean_rel.startswith("~"):
        return ("Error: Absolute paths are not accepted; use a vault-relative path.", True)
    if not clean_rel.endswith(".md"):
        clean_rel += ".md"

    target_file = resolve_note_file(vault, clean_rel)
    if not target_file:
        target_file = contained_path(vault, clean_rel)
        if target_file is None:
            return "Error: Path traversal outside vault boundary is forbidden.", True
        return f"Error: Note '{clean_rel}' does not exist.", True

    with VaultLock(vault):
        content = target_file.read_text(encoding="utf-8")
        replaced_content, ok = replace_markdown_section(content, heading, new_content)
        if not ok:
            return f"Error: Section heading '{heading}' not found in '{clean_rel}'.", True

        fm, body = parse_frontmatter(replaced_content)
        if fm:
            fm["updated"] = datetime.datetime.now().astimezone().isoformat(timespec="seconds")
            fm["updated_by"] = get_machine_id()
            final_content = dump_frontmatter(fm, body)
        else:
            final_content = replaced_content

        tmp_file = target_file.with_name(f".{target_file.name}.tmp.{os.getpid()}")
        tmp_file.write_text(final_content, encoding="utf-8")
        os.replace(tmp_file, target_file)

        try:
            with fts_db_context(vault) as db:
                sync_fts_index(vault, db)
        except Exception:
            pass

    if not clean_rel.startswith("01-Daily"):
        try:
            append_work_log(vault, project="akatsuki", summary=f"replace {clean_rel} #{heading} -> exit 0")
        except Exception:
            pass

    return f"Successfully replaced section '{heading}' in '{clean_rel}'.", False


def write_note(
    vault: Path, rel_path: str, content: str, overwrite: bool = False, raw: bool = False
) -> tuple[str, bool]:
    """Write note or raw config/script into vault safely enforcing boundary, permissions, and kernel lock."""
    clean_rel = rel_path.strip()
    if clean_rel.startswith("/") or clean_rel.startswith("~"):
        return ("Error: Absolute paths are not accepted; use a vault-relative path.", True)

    is_raw = raw or is_raw_path(clean_rel)
    if not is_raw and not clean_rel.endswith(".md"):
        clean_rel += ".md"

    target = contained_path(vault, clean_rel)
    if target is None:
        return "Error: Path traversal outside vault boundary is forbidden.", True

    if is_raw:
        final_content = content
    else:
        ok, _ = validate_note_content(content)
        if ok:
            fm, body = parse_frontmatter(content)
            fm["updated"] = datetime.datetime.now().astimezone().isoformat(timespec="seconds")
            fm["updated_by"] = get_machine_id()
            final_content = dump_frontmatter(fm, body)
        else:
            final_content = auto_heal_frontmatter(content, clean_rel)

    with VaultLock(vault):
        was_existing = target.exists()
        if was_existing and not overwrite:
            return (f"Error: File already exists at '{clean_rel}'. Set overwrite=True to replace.", True)

        target.parent.mkdir(parents=True, exist_ok=True)
        tmp_file = target.with_name(f".{target.name}.tmp.{os.getpid()}")
        tmp_file.write_text(final_content, encoding="utf-8")
        if is_raw and (clean_rel.startswith("60-Scripts/") or clean_rel.endswith((".sh", ".py"))):
            try:
                tmp_file.chmod(0o755)
            except Exception:
                pass
        os.replace(tmp_file, target)

        if not is_raw:
            try:
                with fts_db_context(vault) as db:
                    sync_fts_index(vault, db)
            except Exception:
                pass

    if not clean_rel.startswith("01-Daily"):
        try:
            action = "overwrite" if was_existing else "create"
            append_work_log(vault, project="akatsuki", summary=f"{action} {clean_rel} -> exit 0")
        except Exception:
            pass

    if is_raw:
        return f"Successfully wrote raw file '{clean_rel}'.", False

    v_ok, broken = verify_links(vault)
    report = f"Successfully wrote '{clean_rel}'."
    if not v_ok:
        report += f" Warning: {len(broken)} broken link(s) detected in vault."
    return report, False


def set_note_property(vault: Path, rel_path: str, keypath: str, value_str: str) -> tuple[str, bool]:
    """Surgically update a frontmatter key without rewriting note body."""
    target_file = resolve_note_file(vault, rel_path)
    if not target_file:
        return f"Error: Note '{rel_path}' not found in vault.", True

    with VaultLock(vault):
        content = target_file.read_text(encoding="utf-8")
        fm, body = parse_frontmatter(content)

        try:
            parsed_val = json.loads(value_str)
        except Exception:
            parsed_val = value_str

        keys = keypath.split(".")
        curr = fm
        for k in keys[:-1]:
            if k not in curr or not isinstance(curr[k], dict):
                curr[k] = {}
            curr = curr[k]

        curr[keys[-1]] = parsed_val
        fm["updated"] = datetime.datetime.now().astimezone().isoformat(timespec="seconds")
        fm["updated_by"] = get_machine_id()

        new_content = dump_frontmatter(fm, body)
        tmp_file = target_file.with_name(f".{target_file.name}.tmp.{os.getpid()}")
        tmp_file.write_text(new_content, encoding="utf-8")
        os.replace(tmp_file, target_file)

        try:
            with fts_db_context(vault) as db:
                sync_fts_index(vault, db)
        except Exception:
            pass

    try:
        append_work_log(
            vault,
            project="akatsuki",
            summary=f"set {target_file.name} {keypath}={value_str} -> exit 0",
        )
    except Exception:
        pass

    return f"Successfully updated '{keypath}' in '{target_file.name}'.", False


def list_notes_in_vault(vault: Path, domain: str | None = None) -> list[dict]:
    """List notes in vault with metadata."""
    notes = []
    for f in sorted(vault.glob("**/*.md")):
        rel = str(f.relative_to(vault))
        if rel.startswith("_templates") or rel.startswith("."):
            continue
        if domain and not rel.startswith(domain):
            continue
        try:
            text = f.read_text(encoding="utf-8")
            fm, _ = parse_frontmatter(text)
            notes.append(
                {
                    "rel_path": rel,
                    "stem": f.stem,
                    "title": fm.get("title") or f.stem,
                    "type": fm.get("type") or "note",
                    "summary": fm.get("summary") or "",
                    "status": fm.get("status") or "",
                    "tags": fm.get("tags") or [],
                }
            )
        except Exception:
            continue
    return notes
