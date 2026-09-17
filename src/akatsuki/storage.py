"""Storage, vault resolution, frontmatter parsing, and locking primitives."""

import datetime
import json
import os
import socket
import sys
from pathlib import Path

from akatsuki.constants import (
    HAVE_FCNTL,
    HAVE_PYYAML,
    RAW_EXTS,
)

if HAVE_FCNTL:
    import fcntl

if HAVE_PYYAML:
    import yaml


def get_machine_id() -> str:
    """Return resolved hostname or environment override identifier."""
    return os.environ.get("AKATSUKI_HOST") or os.environ.get("HOSTNAME") or socket.gethostname().split(".")[0]


def _yaml_format_scalar(val: object) -> str:
    """Safely format a scalar value for YAML without unquoted colon defects."""
    if val is None:
        return ""
    if isinstance(val, bool):
        return "true" if val else "false"
    if isinstance(val, (int, float)):
        return str(val)
    s = str(val)
    if not s:
        return '""'
    needs_quotes = (
        ": " in s
        or s.endswith(":")
        or s.startswith(("- ", "#", "[", "{", "*", "&", "!", "|", ">", "%", "@", "`"))
        or any(c in s for c in ('"', "'", "\n"))
        or s.lower() in ("true", "false", "yes", "no", "null", "none")
    )
    if needs_quotes:
        escaped = s.replace("\\", "\\\\").replace('"', '\\"')
        return f'"{escaped}"'
    return s


def dump_frontmatter(fm: dict, body: str) -> str:
    """Serialize YAML frontmatter dictionary and prepend to body."""
    if HAVE_PYYAML:
        new_fm_str = yaml.dump(fm, sort_keys=False).strip()
    else:
        new_fm_lines = []
        for k, v in fm.items():
            if isinstance(v, list):
                new_fm_lines.append(f"{k}:")
                for item in v:
                    new_fm_lines.append(f"  - {_yaml_format_scalar(item)}")
            elif isinstance(v, dict):
                new_fm_lines.append(f"{k}:")
                for sub_k, sub_v in v.items():
                    if isinstance(sub_v, list):
                        new_fm_lines.append(f"  {sub_k}:")
                        for sub_item in sub_v:
                            new_fm_lines.append(f"    - {_yaml_format_scalar(sub_item)}")
                    elif isinstance(sub_v, dict):
                        new_fm_lines.append(f"  {sub_k}: {json.dumps(sub_v, default=str)}")
                    else:
                        new_fm_lines.append(f"  {sub_k}: {_yaml_format_scalar(sub_v)}")
            else:
                new_fm_lines.append(f"{k}: {_yaml_format_scalar(v)}")
        new_fm_str = "\n".join(new_fm_lines)
    return f"---\n{new_fm_str}\n---\n" + body.lstrip()


def is_raw_path(rel_path: str) -> bool:
    name = Path(rel_path).name.lower()
    if name in {"dockerfile", "caddyfile", "makefile"} or name.endswith(".example"):
        return True
    return any(name.endswith(ext) for ext in RAW_EXTS)


CURRENT_VAULT_OVERRIDE: Path | None = None


def resolve_vault_path(explicit_path: str | Path | None = None) -> Path:
    """Multi-tiered vault resolution:
    1. Explicitly passed argument (--vault / param)
    2. CURRENT_VAULT_OVERRIDE (set via CLI --vault, core, or tests)
    3. AKATSUKI_VAULT environment variable
    4. Upward directory walk from current working directory
    5. Well-known fallback paths (~/.config/akatsuki, ~/.akatsuki, ~/akatsuki)
    6. Current working directory fallback
    """
    if explicit_path:
        return Path(explicit_path).expanduser().resolve()

    # Check if core.CURRENT_VAULT_OVERRIDE or local CURRENT_VAULT_OVERRIDE is set
    try:
        import akatsuki.core as _core
        if getattr(_core, "CURRENT_VAULT_OVERRIDE", None) is not None:
            return _core.CURRENT_VAULT_OVERRIDE
    except (ImportError, AttributeError):
        pass

    if CURRENT_VAULT_OVERRIDE is not None:
        return CURRENT_VAULT_OVERRIDE

    env_vault = os.environ.get("AKATSUKI_VAULT")
    if not env_vault:
        cfg_env = Path.home() / ".config" / "knowledge-base" / "env"
        if cfg_env.is_file():
            try:
                for line in cfg_env.read_text(encoding="utf-8").splitlines():
                    clean = line.strip()
                    if clean.startswith("export AKATSUKI_VAULT=") or clean.startswith("AKATSUKI_VAULT="):
                        val = clean.split("=", 1)[1].strip().strip('"').strip("'")
                        val = os.path.expandvars(val)
                        if val and Path(val).is_dir():
                            env_vault = val
                            break
                    elif clean.startswith("export KNOWLEDGE_BASE_DIR=") or clean.startswith("KNOWLEDGE_BASE_DIR="):
                        val = clean.split("=", 1)[1].strip().strip('"').strip("'")
                        val = os.path.expandvars(val)
                        if val and (Path(val) / "akatsuki").is_dir():
                            env_vault = str(Path(val) / "akatsuki")
                            break
            except Exception:
                pass

    if env_vault:
        return Path(env_vault).expanduser().resolve()

    # Upward directory walk
    try:
        cwd = Path.cwd().resolve()
        for parent in [cwd, *cwd.parents]:
            if (parent / "akatsuki" / "40-Systems").is_dir() and (parent / "akatsuki" / "20-Projects").is_dir():
                return (parent / "akatsuki").resolve()
            if (parent / ".akatsuki").is_dir() or (
                (parent / "INDEX.md").is_file() and (parent / "AGENTS.md").is_file()
            ):
                return parent
            if (parent / "40-Systems").is_dir() and (parent / "20-Projects").is_dir():
                return parent
    except Exception:
        pass

    # Well-known system locations
    home = Path.home()
    for candidate in [
        home / ".config" / "akatsuki",
        home / ".akatsuki",
        home / "akatsuki",
    ]:
        if candidate.exists() and candidate.is_dir():
            return candidate.resolve()

    return Path.cwd().resolve()


def get_vault() -> Path:
    vault = resolve_vault_path()
    if not vault.exists():
        sys.stderr.write(
            f"Error: akatsuki vault path not found at {vault}\n"
            "Set $AKATSUKI_VAULT, specify --vault, or run 'akatsuki init' to bootstrap a new vault.\n"
        )
        sys.exit(1)
    return vault


def contained_path(vault: Path, rel_path: str) -> Path | None:
    """Resolve rel_path strictly inside the vault. Returns None on escape or root."""
    clean = rel_path.strip()
    if not clean or clean in (".", "./"):
        return None
    candidate = Path(clean)
    if candidate.is_absolute():
        return None
    vault = vault.resolve()
    target = (vault / candidate).resolve()
    if target == vault or vault not in target.parents:
        return None
    return target


def parse_frontmatter(content: str) -> tuple[dict[str, object], str]:
    """Extract frontmatter and body from markdown content."""
    if not content.startswith("---"):
        return {}, content

    parts = content.split("---", 2)
    if len(parts) < 3:
        return {}, content

    fm_raw = parts[1]
    body = parts[2]

    if HAVE_PYYAML:
        try:
            data = yaml.safe_load(fm_raw)
            if isinstance(data, dict):
                return data, body
        except Exception:
            pass

    metadata: dict[str, object] = {}
    current_top_key: str | None = None
    current_sub_key: str | None = None

    for line in fm_raw.splitlines():
        if not line.strip() or line.strip().startswith("#"):
            continue
        indent = len(line) - len(line.lstrip(" "))
        stripped = line.strip()

        if indent == 0:
            if ":" in stripped:
                k, v = stripped.split(":", 1)
                k, v = k.strip(), v.strip()
                current_top_key = k
                current_sub_key = None
                if not v:
                    metadata[k] = None
                elif v.startswith("[") and v.endswith("]"):
                    metadata[k] = [x.strip().strip('"').strip("'") for x in v[1:-1].split(",") if x.strip()]
                else:
                    metadata[k] = v.strip('"').strip("'")
        elif indent == 2:
            if stripped.startswith("- "):
                val = stripped[2:].strip().strip('"').strip("'")
                if current_top_key:
                    if metadata.get(current_top_key) is None or not isinstance(metadata.get(current_top_key), list):
                        metadata[current_top_key] = []
                    assert isinstance(metadata[current_top_key], list)
                    metadata[current_top_key].append(val)
            elif ":" in stripped:
                sub_k, sub_v = stripped.split(":", 1)
                sub_k, sub_v = sub_k.strip(), sub_v.strip()
                if current_top_key:
                    if metadata.get(current_top_key) is None or not isinstance(metadata.get(current_top_key), dict):
                        metadata[current_top_key] = {}
                    current_sub_key = sub_k
                    assert isinstance(metadata[current_top_key], dict)
                    if not sub_v:
                        metadata[current_top_key][sub_k] = []
                    elif sub_v.startswith("[") and sub_v.endswith("]"):
                        metadata[current_top_key][sub_k] = [
                            x.strip().strip('"').strip("'") for x in sub_v[1:-1].split(",") if x.strip()
                        ]
                    else:
                        metadata[current_top_key][sub_k] = sub_v.strip('"').strip("'")
        elif indent >= 4:
            if stripped.startswith("- "):
                val = stripped[2:].strip().strip('"').strip("'")
                if current_top_key and current_sub_key:
                    top_dict = metadata.get(current_top_key)
                    if isinstance(top_dict, dict):
                        if not isinstance(top_dict.get(current_sub_key), list):
                            top_dict[current_sub_key] = []
                        top_dict[current_sub_key].append(val)

    for k, v in metadata.items():
        if v is None:
            metadata[k] = []

    return metadata, body


def validate_frontmatter_yaml(fm_raw: str) -> list[str]:
    """Validate YAML frontmatter syntax without requiring PyYAML, falling back to PyYAML if available."""
    errors = []
    if HAVE_PYYAML:
        try:
            yaml.safe_load(fm_raw)
            return []
        except Exception as e:
            return [f"YAML parser error: {e}"]

    # Built-in strict YAML sanity checks (zero-dependency)
    for idx, line in enumerate(fm_raw.splitlines(), 1):
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        if stripped.startswith("- "):
            val = stripped[2:].strip()
            if ": " in val and not (
                (val.startswith('"') and val.endswith('"')) or (val.startswith("'") and val.endswith("'"))
            ):
                errors.append(f"Line {idx}: Unquoted colon in list item: {stripped}")
            continue
        if ":" not in stripped:
            errors.append(f"Line {idx}: Missing key-value colon delimiter: {stripped}")
            continue
        _k, v = stripped.split(":", 1)
        val = v.strip()
        if not val:
            continue
        if ": " in val and not (
            (val.startswith('"') and val.endswith('"')) or (val.startswith("'") and val.endswith("'"))
        ):
            errors.append(f"Line {idx}: Unquoted colon in scalar value: {stripped}")
        elif (val.startswith('"') and not val.endswith('"')) or (val.startswith("'") and not val.endswith("'")):
            errors.append(f"Line {idx}: Unterminated quote: {stripped}")
    return errors


def resolve_note_file(vault: Path, query: str) -> Path | None:
    """Resolve a note path by exact path, relative path, or stem strictly inside vault."""
    q = query.strip()
    if q.startswith("/") or q.startswith("~"):
        return None
    if q.endswith(".md"):
        q_stem = q[:-3]
    else:
        q_stem = q

    # 1. Exact path (strictly contained within vault)
    p = contained_path(vault, q)
    if p is not None and p.is_file():
        return p
    p_md = contained_path(vault, f"{q}.md")
    if p_md is not None and p_md.is_file():
        return p_md

    # 2. Match stem or path suffix within vault only
    candidates = []
    for f in vault.glob("**/*.md"):
        rel = str(f.relative_to(vault).with_suffix(""))
        if f.stem.lower() == q_stem.lower() or rel.lower() == q_stem.lower():
            candidates.append(f)

    if len(candidates) == 1:
        return candidates[0]
    elif len(candidates) > 1:
        for c in candidates:
            if c.stem == q_stem or str(c.relative_to(vault).with_suffix("")) == q_stem:
                return c
        return candidates[0]

    return None


class VaultLock:
    """Process-safe kernel advisory lock for concurrent multi-agent mutations."""

    def __init__(self, vault: Path):
        self.lock_path = vault / ".akatsuki.lock"
        self._fd = None

    def __enter__(self):
        try:
            self.lock_path.parent.mkdir(parents=True, exist_ok=True)
            self._fd = open(self.lock_path, "a")
            if HAVE_FCNTL:
                fcntl.flock(self._fd.fileno(), fcntl.LOCK_EX)
        except Exception as e:
            if self._fd:
                try:
                    self._fd.close()
                except Exception:
                    pass
                self._fd = None
            raise RuntimeError(f"VaultLock acquisition failed for {self.lock_path}: {e}") from e
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        if self._fd:
            try:
                if HAVE_FCNTL:
                    fcntl.flock(self._fd.fileno(), fcntl.LOCK_UN)
            finally:
                self._fd.close()
                self._fd = None


def ensure_daily_note(vault: Path, date_str: str) -> Path:
    """Ensure the daily note for date_str exists, bootstrapping from template or default."""
    daily_dir = vault / "01-Daily"
    daily_dir.mkdir(parents=True, exist_ok=True)
    daily_file = daily_dir / f"{date_str}.md"

    if not daily_file.exists():
        template_file = vault / "_templates" / "Daily-Template.md"
        if template_file.exists():
            tmpl = template_file.read_text(encoding="utf-8")
            content = tmpl.replace("{{date}}", date_str).replace("{{title}}", f"Daily Log: {date_str}")
        else:
            dev = get_machine_id()
            now_iso = datetime.datetime.now().astimezone().isoformat(timespec="seconds")
            content = (
                f"---\n"
                f"title: \"Daily Log: {date_str}\"\n"
                f"date: {date_str}\n"
                f"type: daily\n"
                f"tags:\n"
                f"  - daily\n"
                f"summary: \"Operational work log and agent horizon for {date_str}.\"\n"
                f"updated: {now_iso}\n"
                f"updated_by: {dev}\n"
                f"---\n\n"
                f"# Daily Log: {date_str}\n\n"
                f"## 🎯 Focus Horizons\n"
                f"- [ ] Establish daily operating horizon\n\n"
                f"## 📝 Work Log & Session Notes\n"
            )
        daily_file.write_text(content, encoding="utf-8")

    return daily_file


def validate_note_content(content: str) -> tuple[bool, str]:
    """Validate that note content has required frontmatter fields."""
    if not content.startswith("---"):
        return False, "Note must start with YAML frontmatter delimiter '---'."
    parts = content.split("---", 2)
    if len(parts) < 3:
        return False, "Note frontmatter is not closed with '---'."
    fm, _ = parse_frontmatter(content)
    note_type = str(fm.get("type", "note"))
    required = ["title", "date", "type", "tags", "summary"]
    if note_type == "project":
        required.append("status")
    missing = [f for f in required if f not in fm or not fm.get(f)]
    if missing:
        return False, f"Missing required frontmatter field(s): {', '.join(missing)}"
    return True, "Valid"


def auto_heal_frontmatter(content: str, rel_path: str) -> str:
    """Ensure markdown content has valid YAML frontmatter."""
    clean_rel = rel_path.strip().lstrip("/")
    stem = Path(clean_rel).stem
    date_str = datetime.datetime.now().astimezone().strftime("%Y-%m-%d")

    if clean_rel.startswith("20-Projects"):
        inferred_type = "project"
    elif clean_rel.startswith("40-Systems"):
        inferred_type = "system"
    elif clean_rel.startswith("30-Agents"):
        inferred_type = "agent"
    elif clean_rel.startswith("01-Daily"):
        inferred_type = "daily"
    elif clean_rel.startswith("90-Reference"):
        inferred_type = "reference"
    elif clean_rel.startswith("90-Database"):
        inferred_type = "database"
    else:
        inferred_type = "note"

    if content.startswith("---") and len(content.split("---", 2)) >= 3:
        fm, body = parse_frontmatter(content)
    else:
        fm, body = {}, content

    title = fm.get("title")
    if not title:
        for line in body.splitlines():
            if line.strip().startswith("# "):
                title = line.strip().lstrip("#").strip()
                break
        if not title:
            title = stem

    date_val = fm.get("date", date_str)
    type_val = fm.get("type", inferred_type)
    tags = fm.get("tags") or [inferred_type]
    if isinstance(tags, str):
        tags = [t.strip() for t in tags.split(",") if t.strip()]
    summary_val = fm.get("summary") or f"{title} overview and operational documentation."
    now_iso = datetime.datetime.now().astimezone().isoformat(timespec="seconds")
    dev = get_machine_id()
    updated_val = fm.get("updated", now_iso)
    updated_by_val = fm.get("updated_by", dev)

    merged_fm = dict(fm)
    merged_fm["title"] = title
    merged_fm["date"] = date_val
    merged_fm["type"] = type_val
    if type_val == "project" and "status" not in merged_fm:
        merged_fm["status"] = "active"
    merged_fm["tags"] = tags
    merged_fm["summary"] = summary_val
    merged_fm["updated"] = updated_val
    merged_fm["updated_by"] = updated_by_val

    return dump_frontmatter(merged_fm, body.lstrip())
