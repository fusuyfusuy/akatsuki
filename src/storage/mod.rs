//! Storage substrate: Vault discovery, path containment, atomic I/O, frontmatter, and locking.

use anyhow::{Context, Result};
use fs2::FileExt;
use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use crate::constants::RAW_EXTS;

pub struct VaultLock {
    _file: File,
}

impl VaultLock {
    pub fn acquire(vault: &Path) -> Result<Self> {
        let lock_path = vault.join(".akatsuki.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| format!("Failed to open lockfile: {}", lock_path.display()))?;

        file.lock_exclusive().with_context(|| {
            format!(
                "Failed to acquire exclusive lock on {}",
                lock_path.display()
            )
        })?;

        Ok(Self { _file: file })
    }
}

pub fn is_raw_path(rel_path: &str) -> bool {
    let lower = rel_path.to_lowercase();
    if lower == "dockerfile"
        || lower == "caddyfile"
        || lower == "makefile"
        || lower.ends_with(".example")
    {
        return true;
    }
    RAW_EXTS.iter().any(|ext| lower.ends_with(ext))
}

/// Identity of the host performing a mutation: `AKATSUKI_HOST`, then `HOSTNAME`,
/// then the short system hostname.
pub fn machine_id() -> String {
    for key in ["AKATSUKI_HOST", "HOSTNAME"] {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim().to_string();
            if !value.is_empty() {
                return value;
            }
        }
    }
    if let Ok(output) = std::process::Command::new("hostname").output() {
        if let Ok(name) = String::from_utf8(output.stdout) {
            let name = name.trim();
            if !name.is_empty() {
                return name.split('.').next().unwrap_or(name).to_string();
            }
        }
    }
    "local".to_string()
}

pub fn resolve_vault_path(explicit: Option<&Path>) -> PathBuf {
    if let Some(p) = explicit {
        if let Ok(canon) = p.canonicalize() {
            return canon;
        }
        return p.to_path_buf();
    }

    if let Ok(env_p) = std::env::var("AKATSUKI_VAULT") {
        let pb = PathBuf::from(shellexpand(&env_p));
        if pb.is_dir() {
            if let Ok(c) = pb.canonicalize() {
                return c;
            }
            return pb;
        }
    }

    // Check ~/.config/knowledge-base/env
    if let Some(home) = dirs::home_dir() {
        let cfg_env = home.join(".config/knowledge-base/env");
        if cfg_env.is_file() {
            if let Ok(content) = fs::read_to_string(&cfg_env) {
                for line in content.lines() {
                    let trim = line.trim();
                    if let Some(val) = trim
                        .strip_prefix("export AKATSUKI_VAULT=")
                        .or_else(|| trim.strip_prefix("AKATSUKI_VAULT="))
                    {
                        let clean = val.trim_matches(|c| c == '"' || c == '\'');
                        let pb = PathBuf::from(shellexpand(clean));
                        if pb.is_dir() {
                            return pb.canonicalize().unwrap_or(pb);
                        }
                    }
                }
            }
        }
    }

    // Upward directory walk
    if let Ok(cwd) = std::env::current_dir() {
        let mut curr: Option<&Path> = Some(&cwd);
        while let Some(dir) = curr {
            if dir.join("40-Systems").is_dir() && dir.join("20-Projects").is_dir() {
                return dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
            }
            if dir.join("akatsuki/40-Systems").is_dir() && dir.join("akatsuki/20-Projects").is_dir()
            {
                let ak = dir.join("akatsuki");
                return ak.canonicalize().unwrap_or(ak);
            }
            if dir.join(".akatsuki").is_dir()
                || (dir.join("INDEX.md").is_file() && dir.join("AGENTS.md").is_file())
            {
                return dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
            }
            curr = dir.parent();
        }
    }

    // Well-known defaults
    if let Some(home) = dirs::home_dir() {
        let candidates = [
            home.join("configs/knowledge-base/akatsuki"),
            home.join(".config/akatsuki"),
            home.join(".akatsuki"),
            home.join("akatsuki"),
        ];
        for c in candidates {
            if c.is_dir() {
                return c.canonicalize().unwrap_or(c);
            }
        }
    }

    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

pub fn shellexpand(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;

    while let Some(idx) = rest.find('$') {
        out.push_str(&rest[..idx]);
        let after_dollar = idx + 1;
        let tail = &rest[after_dollar..];

        let (name, consumed) = if let Some(braced) = tail.strip_prefix('{') {
            match braced.find('}') {
                Some(end) => (&braced[..end], end + 2),
                None => {
                    out.push('$');
                    rest = tail;
                    continue;
                }
            }
        } else {
            let len = tail
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .count();
            if len == 0 {
                out.push('$');
                rest = tail;
                continue;
            }
            (&tail[..len], len)
        };

        match std::env::var(name) {
            Ok(value) => out.push_str(&value),
            Err(_) => out.push_str(&rest[idx..after_dollar + consumed]),
        }
        rest = &rest[after_dollar + consumed..];
    }

    out.push_str(rest);
    out
}

pub fn contained_path(vault: &Path, rel_path: &str) -> Option<PathBuf> {
    let clean = rel_path.trim().trim_start_matches('/');
    if clean.is_empty() || clean == "." || clean == "./" || clean.contains('\0') {
        return None;
    }
    let candidate = vault.join(clean);
    let norm_candidate = normalize_path(&candidate);
    let norm_vault = normalize_path(vault);

    if !norm_candidate.starts_with(&norm_vault) || norm_candidate == norm_vault {
        return None;
    }

    let components: Vec<std::path::Component> = norm_candidate
        .strip_prefix(&norm_vault)
        .ok()?
        .components()
        .collect();

    // Disallow hidden directory traversals (e.g. .git/hooks)
    for comp in &components {
        if let std::path::Component::Normal(c) = comp {
            if let Some(s) = c.to_str() {
                if s.starts_with('.') && s != ".akatsuki" && s != ".akatsuki.lock" {
                    return None;
                }
            }
        }
    }

    // A symlink inside the vault must never resolve outside of it: containment is
    // enforced against the real target, not the lexical path.
    let mut resolved = norm_vault.clone();
    for comp in components {
        resolved.push(comp);
        if fs::symlink_metadata(&resolved).is_ok_and(|m| m.file_type().is_symlink()) {
            resolved = fs::canonicalize(&resolved).ok()?;
            if !resolved.starts_with(&norm_vault) {
                return None;
            }
        }
    }

    Some(resolved)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for comp in path.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                components.pop();
            }
            c => components.push(c),
        }
    }
    components.into_iter().collect()
}

/// Splits a note into its YAML frontmatter and body.
///
/// A note without a frontmatter fence yields empty metadata plus the untouched
/// text. A note whose fence is present but whose YAML does not parse is an error
/// — degrading to empty metadata silently rewrites the note into corruption on
/// the next mutation.
pub fn parse_frontmatter(content: &str) -> Result<(Value, String)> {
    let text = content.strip_prefix('\u{feff}').unwrap_or(content);
    let after_open = match opening_fence(text) {
        Some(rest) => rest,
        None => return Ok((serde_json::json!({}), content.to_string())),
    };

    let mut offset = 0usize;
    let mut close: Option<(usize, usize)> = None;
    for line in after_open.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']).trim() == "---" {
            close = Some((offset, offset + line.len()));
            break;
        }
        offset += line.len();
    }

    let (yaml_end, body_start) =
        close.context("Malformed frontmatter: opening '---' fence is never closed")?;

    let parsed: Value = serde_yaml::from_str(&after_open[..yaml_end])
        .context("Invalid YAML frontmatter: value is not parseable")?;

    let meta = match parsed {
        Value::Null => serde_json::json!({}),
        Value::Object(map) => Value::Object(map),
        other => anyhow::bail!(
            "Invalid YAML frontmatter: expected a mapping, found {}",
            match other {
                Value::Array(_) => "a sequence",
                Value::String(_) => "a string",
                Value::Number(_) => "a number",
                Value::Bool(_) => "a boolean",
                _ => "a scalar",
            }
        ),
    };

    let body = after_open[body_start..]
        .trim_start_matches('\n')
        .to_string();
    Ok((meta, body))
}

/// Text following the opening `---` fence line, or `None` when the note does not
/// begin with a frontmatter fence.
fn opening_fence(content: &str) -> Option<&str> {
    let line_end = content.find('\n')?;
    if content[..line_end].trim_end_matches('\r').trim() != "---" {
        return None;
    }
    Some(&content[line_end + 1..])
}

pub fn dump_frontmatter(meta: &Value, body: &str) -> String {
    if meta.as_object().is_none_or(|m| m.is_empty()) {
        return body.to_string();
    }
    let yaml_str = serde_yaml::to_string(meta).unwrap_or_default();
    format!("---\n{}---\n{}", yaml_str, body)
}

pub fn write_atomic(target: &Path, content: &str) -> Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let pid = std::process::id();
    let rand_id = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let tmp_path = target.with_extension(format!("tmp.{}.{}", pid, rand_id));

    fs::write(&tmp_path, content)?;
    fs::rename(&tmp_path, target).with_context(|| {
        let _ = fs::remove_file(&tmp_path);
        format!(
            "Failed to atomically rename tmp file to {}",
            target.display()
        )
    })?;

    Ok(())
}

pub fn resolve_note_file(vault: &Path, query: &str) -> Option<PathBuf> {
    let q = query.trim().trim_matches(|c| c == '\'' || c == '"');
    if q.is_empty() || q.starts_with('/') || q.starts_with('~') {
        return None;
    }

    let clean_q = q.strip_suffix(".md").unwrap_or(q);

    // 1. Exact path inside vault
    if let Some(p) = contained_path(vault, q) {
        if p.is_file() {
            return Some(p);
        }
    }
    let with_md = format!("{}.md", clean_q);
    if let Some(p) = contained_path(vault, &with_md) {
        if p.is_file() {
            return Some(p);
        }
    }

    // 2. Normalized space <-> hyphen candidates
    let hyphenated = clean_q.replace(' ', "-");
    let spaced = clean_q.replace('-', " ");

    if let Some(p) = contained_path(vault, &format!("{}.md", hyphenated)) {
        if p.is_file() {
            return Some(p);
        }
    }

    // 3. Fast SQLite cache lookup if index exists
    let cache_db = vault.join(".akatsuki/cache.db");
    if cache_db.is_file() {
        if let Ok(con) = rusqlite::Connection::open_with_flags(
            &cache_db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        ) {
            let sql = "SELECT rel_path FROM entities WHERE stem = ?1 COLLATE NOCASE OR stem = ?2 COLLATE NOCASE OR title = ?1 COLLATE NOCASE OR title = ?3 COLLATE NOCASE LIMIT 1";
            if let Ok(mut stmt) = con.prepare(sql) {
                if let Ok(rel) = stmt
                    .query_row(rusqlite::params![clean_q, hyphenated, spaced], |row| {
                        row.get::<_, String>(0)
                    })
                {
                    if let Some(p) = contained_path(vault, &rel) {
                        if p.is_file() {
                            return Some(p);
                        }
                    }
                }
            }
        }
    }

    // 4. Filesystem scan fallback comparing stems
    let ignored_dirs = [
        ".git",
        ".akatsuki",
        ".venv",
        "node_modules",
        ".obsidian",
        "__pycache__",
        "_templates",
    ];
    let walker = walkdir::WalkDir::new(vault).into_iter().filter_entry(|e| {
        let name = e.file_name().to_string_lossy();
        !ignored_dirs.iter().any(|ig| *ig == name)
    });

    let q_stem_lower = clean_q.to_lowercase();
    let q_hyphen_lower = hyphenated.to_lowercase();
    let q_space_lower = spaced.to_lowercase();

    for entry in walker.flatten() {
        if entry.file_type().is_file() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "md") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    let stem_lower = stem.to_lowercase();
                    let stem_space = stem_lower.replace('-', " ");
                    if stem_lower == q_stem_lower
                        || stem_lower == q_hyphen_lower
                        || stem_space == q_space_lower
                        || stem_space == q_stem_lower
                    {
                        return Some(path.to_path_buf());
                    }
                }
            }
        }
    }

    None
}

pub fn normalize_heading(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '-' || *c == '_')
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Level and normalized text of a markdown heading line.
pub fn heading_at(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim();
    if !trimmed.starts_with('#') {
        return None;
    }
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    Some((level, normalize_heading(&trimmed[level..])))
}

/// Locates a section by heading: exact normalized match first, substring second.
///
/// Returns `(heading_line, heading_level, end_line)` where `end_line` is the
/// first following heading of the same or higher level, or the end of the input.
/// Every reader and writer of sections goes through this so that `read --section`
/// and `replace --heading` agree on what a heading is.
pub fn locate_section(lines: &[&str], heading: &str) -> Option<(usize, usize, usize)> {
    let target = normalize_heading(heading);
    if target.is_empty() {
        return None;
    }

    let mut found: Option<(usize, usize)> = None;
    for (index, line) in lines.iter().enumerate() {
        let Some((level, name)) = heading_at(line) else {
            continue;
        };
        match found {
            None => {
                if name == target || name.contains(&target) {
                    found = Some((index, level));
                }
            }
            Some((_, matched_level)) if level <= matched_level => {
                let (start, start_level) = found.expect("checked");
                return Some((start, start_level, index));
            }
            Some(_) => {}
        }
    }

    found.map(|(start, level)| (start, level, lines.len()))
}

pub fn extract_section(content: &str, section_name: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let (start, _level, end) = locate_section(&lines, section_name)?;
    Some(lines[start..end].join("\n"))
}

/// Note type inferred from the domain directory, mirroring the vault layout.
pub fn infer_note_type(rel_path: &str) -> &'static str {
    let clean = rel_path.trim().trim_start_matches('/');
    if clean.starts_with("20-Projects") {
        "project"
    } else if clean.starts_with("40-Systems") {
        "system"
    } else if clean.starts_with("30-Agents") {
        "agent"
    } else if clean.starts_with("01-Daily") {
        "daily"
    } else if clean.starts_with("90-Reference") {
        "reference"
    } else if clean.starts_with("90-Database") {
        "database"
    } else {
        "note"
    }
}

/// Completes a note's frontmatter so a freshly written note satisfies the strict
/// lint gate, preserving every key the caller already supplied.
///
/// Malformed frontmatter is an error: healing must never rewrite metadata it
/// could not read.
pub fn auto_heal_frontmatter(content: &str, rel_path: &str) -> Result<String> {
    let clean = rel_path.trim().trim_start_matches('/');
    let stem = Path::new(clean)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(clean)
        .to_string();
    let (mut fm, body) = parse_frontmatter(content)?;
    let inferred_type = infer_note_type(clean);
    let now = chrono::Local::now();

    let heading_title = body
        .lines()
        .find_map(|line| line.trim().strip_prefix("# ").map(|t| t.trim().to_string()));

    let existing_text = |map: &serde_json::Map<String, Value>, key: &str| -> Option<String> {
        map.get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };

    let map = fm
        .as_object()
        .context("Frontmatter must be a YAML mapping")?;
    let title = existing_text(map, "title")
        .or(heading_title)
        .unwrap_or_else(|| stem.clone());
    let date = existing_text(map, "date").unwrap_or_else(|| now.format("%Y-%m-%d").to_string());
    let note_type = existing_text(map, "type").unwrap_or_else(|| inferred_type.to_string());
    let summary = existing_text(map, "summary")
        .unwrap_or_else(|| format!("{} overview and operational documentation.", title));
    let updated = existing_text(map, "updated")
        .unwrap_or_else(|| now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    let updated_by = existing_text(map, "updated_by").unwrap_or_else(machine_id);
    let tags = match map.get("tags") {
        Some(Value::Array(items)) if !items.is_empty() => Value::Array(items.clone()),
        Some(Value::String(raw)) => Value::Array(
            raw.split(',')
                .map(|t| t.trim())
                .filter(|t| !t.is_empty())
                .map(|t| Value::String(t.to_string()))
                .collect(),
        ),
        _ => json!([inferred_type]),
    };
    let status = match (note_type.as_str(), existing_text(map, "status")) {
        ("project", None) => Some(Value::String("active".to_string())),
        _ => None,
    };

    let map = fm.as_object_mut().expect("checked mapping");
    map.insert("title".to_string(), Value::String(title));
    map.insert("date".to_string(), Value::String(date));
    map.insert("type".to_string(), Value::String(note_type));
    map.insert("tags".to_string(), tags);
    map.insert("summary".to_string(), Value::String(summary));
    map.insert("updated".to_string(), Value::String(updated));
    map.insert("updated_by".to_string(), Value::String(updated_by));
    if let Some(status) = status {
        map.insert("status".to_string(), status);
    }

    Ok(dump_frontmatter(&fm, body.trim_start_matches('\n')))
}
/// Packs markdown into a heuristic token budget (~4 chars per token), always
/// keeping the frontmatter block intact and marking the cut explicitly.
pub fn apply_token_budget(text: &str, budget: Option<usize>) -> String {
    let Some(budget) = budget.filter(|b| *b > 0) else {
        return text.to_string();
    };
    let char_budget = budget * 4;
    if text.len() <= char_budget {
        return text.to_string();
    }

    let mut packed = String::new();
    let mut used = 0usize;
    let mut in_frontmatter = text.starts_with("---");
    let mut fences = 0usize;
    let mut frontmatter_chars = 0usize;

    for line in text.split_inclusive('\n') {
        if in_frontmatter {
            packed.push_str(line);
            used += line.len();
            frontmatter_chars += line.len();
            if line.trim() == "---" {
                fences += 1;
                if fences == 2 {
                    in_frontmatter = false;
                }
            }
            continue;
        }

        let effective_limit = char_budget.max(frontmatter_chars + 120);
        if used + line.len() > effective_limit {
            packed.push_str(&format!(
                "\n[Notice: Output truncated to fit budget of ~{} tokens]\n",
                budget
            ));
            break;
        }
        packed.push_str(line);
        used += line.len();
    }

    packed
}
