//! Storage substrate: Vault discovery, path containment, atomic I/O, frontmatter, and locking.

use anyhow::{Context, Result};
use fs2::FileExt;
use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static ATOMIC_WRITE_COUNTER: AtomicU64 = AtomicU64::new(1);

use crate::constants::{LOCK_TIMEOUT_SECS, RAW_EXTS};

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

        let deadline = Instant::now() + Duration::from_secs(LOCK_TIMEOUT_SECS);
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(Self { _file: file }),
                Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                    if Instant::now() >= deadline {
                        anyhow::bail!(
                            "Vault lock {} held for more than {}s",
                            lock_path.display(),
                            LOCK_TIMEOUT_SECS
                        );
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => {
                    return Err(e).with_context(|| {
                        format!(
                            "Failed to acquire exclusive lock on {}",
                            lock_path.display()
                        )
                    })
                }
            }
        }
    }
}

pub fn is_raw_path(rel_path: &str) -> bool {
    let lower = rel_path.to_lowercase();
    let file_name = Path::new(rel_path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("")
        .to_lowercase();
    if file_name == "dockerfile"
        || file_name == "caddyfile"
        || file_name == "makefile"
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

/// Expand a leading `~`/`~/` using the home directory, then `$VAR` references.
fn expand_path(s: &str) -> PathBuf {
    let expanded = shellexpand(s);
    let home = dirs::home_dir();
    match (expanded.strip_prefix('~'), home) {
        (Some(""), Some(h)) => h,
        (Some(rest), Some(h)) if rest.starts_with('/') => h.join(rest.trim_start_matches('/')),
        _ => PathBuf::from(expanded),
    }
}

fn canonical_dir(p: PathBuf) -> PathBuf {
    p.canonicalize().unwrap_or(p)
}

fn vault_from_config_file(home: &Path) -> Option<PathBuf> {
    let content = fs::read_to_string(home.join(".config/knowledge-base/env")).ok()?;
    content.lines().find_map(|line| {
        let trim = line.trim();
        let val = trim
            .strip_prefix("export AKATSUKI_VAULT=")
            .or_else(|| trim.strip_prefix("AKATSUKI_VAULT="))?;
        let pb = expand_path(val.trim_matches(|c| c == '"' || c == '\''));
        pb.is_dir().then(|| canonical_dir(pb))
    })
}

fn is_vault_marker_dir(dir: &Path) -> Option<PathBuf> {
    if dir.join("40-Systems").is_dir() && dir.join("20-Projects").is_dir() {
        return Some(dir.to_path_buf());
    }
    let av = dir.join("akatsuvault");
    if av.join("40-Systems").is_dir() && av.join("20-Projects").is_dir() {
        return Some(av);
    }
    let ak = dir.join("akatsuki");
    if ak.join("40-Systems").is_dir() && ak.join("20-Projects").is_dir() {
        return Some(ak);
    }
    let marked = dir.join(".akatsuki").is_dir()
        || (dir.join("INDEX.md").is_file() && dir.join("AGENTS.md").is_file());
    marked.then(|| dir.to_path_buf())
}

fn vault_from_cwd_walk(home: Option<&Path>) -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    cwd.ancestors()
        .filter(|dir| home != Some(*dir))
        .find_map(is_vault_marker_dir)
        .map(canonical_dir)
}

/// Resolve the vault root: explicit path, `AKATSUKI_VAULT`, config file, marker walk
/// from the cwd (never `$HOME` itself), then well-known home directories.
/// An explicit or env-provided path that is not a directory is an error; there is
/// no cwd fallback.
pub fn resolve_vault_path(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        let pb = expand_path(&p.to_string_lossy());
        anyhow::ensure!(
            pb.is_dir(),
            "vault path is not a directory: {}",
            pb.display()
        );
        return Ok(canonical_dir(pb));
    }
    if let Ok(env_p) = std::env::var("AKATSUKI_VAULT") {
        let pb = expand_path(&env_p);
        anyhow::ensure!(
            pb.is_dir(),
            "AKATSUKI_VAULT is not a directory: {}",
            pb.display()
        );
        return Ok(canonical_dir(pb));
    }
    let home = dirs::home_dir();
    if let Some(v) = home.as_deref().and_then(vault_from_config_file) {
        return Ok(v);
    }
    if let Some(v) = vault_from_cwd_walk(home.as_deref()) {
        return Ok(v);
    }
    let defaults = [
        "configs/knowledge-base/akatsuvault",
        "configs/knowledge-base/akatsuki",
        ".config/akatsuki",
        ".akatsuki",
        "akatsuki",
    ];
    home.iter()
        .flat_map(|h| defaults.iter().map(move |d| h.join(d)))
        .find(|c| c.is_dir())
        .map(canonical_dir)
        .ok_or_else(|| anyhow::anyhow!("no vault found: pass --vault or set AKATSUKI_VAULT"))
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
    let abs_vault = if vault.is_relative() {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(vault)
    } else {
        vault.to_path_buf()
    };
    let candidate = abs_vault.join(clean);
    let norm_candidate = normalize_path(&candidate);
    let norm_vault = normalize_path(&abs_vault);

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
    let canon_vault = fs::canonicalize(&abs_vault).unwrap_or_else(|_| norm_vault.clone());
    let mut resolved = norm_vault.clone();
    for comp in components {
        resolved.push(comp);
        if fs::symlink_metadata(&resolved).is_ok_and(|m| m.file_type().is_symlink()) {
            resolved = fs::canonicalize(&resolved).ok()?;
            if !resolved.starts_with(&norm_vault) && !resolved.starts_with(&canon_vault) {
                return None;
            }
        }
    }

    Some(resolved)
}

pub(crate) fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for comp in path.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if components.is_empty()
                    || components
                        .iter()
                        .all(|c| matches!(c, std::path::Component::ParentDir))
                {
                    components.push(comp);
                } else if components.last() != Some(&std::path::Component::RootDir) {
                    components.pop();
                }
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

    let (yaml_end, body_start) = closing_fence(after_open)
        .context("Malformed frontmatter: opening '---' fence is never closed")?;

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

/// A frontmatter fence is exactly `---` at column 0 (trailing whitespace allowed).
fn is_fence(line: &str) -> bool {
    line.trim_end() == "---"
}

/// Text following the opening `---` fence line, or `None` when the note does not
/// begin with a frontmatter fence.
fn opening_fence(content: &str) -> Option<&str> {
    let line_end = content.find('\n')?;
    if !is_fence(&content[..line_end]) {
        return None;
    }
    Some(&content[line_end + 1..])
}

/// `(yaml_end, body_start)` offsets of the first column-0 closing fence in `after_open`.
fn closing_fence(after_open: &str) -> Option<(usize, usize)> {
    let mut offset = 0usize;
    for line in after_open.split_inclusive('\n') {
        if is_fence(line) {
            return Some((offset, offset + line.len()));
        }
        offset += line.len();
    }
    None
}

/// `(frontmatter block including both fences, body)` when `text` starts with a
/// well-formed frontmatter block; `None` otherwise (no fence, unclosed, or BOM).
pub fn split_frontmatter(text: &str) -> Option<(&str, &str)> {
    let after_open = opening_fence(text)?;
    let (_, body_start) = closing_fence(after_open)?;
    let block_len = text.len() - after_open.len() + body_start;
    Some((&text[..block_len], &text[block_len..]))
}

/// The note text with a well-formed frontmatter block removed; text with no
/// fence or an unclosed fence is returned whole (lenient, never errors).
fn body_after_frontmatter(content: &str) -> &str {
    let text = content.strip_prefix('\u{feff}').unwrap_or(content);
    let Some(after_open) = opening_fence(text) else {
        return text;
    };
    match closing_fence(after_open) {
        Some((_, body_start)) => &after_open[body_start..],
        None => text,
    }
}

/// Line span `[start, end)` of top-level `key` in `lines` (key line plus its
/// indented / sequence continuation lines). `Err` when the YAML uses a shape this
/// patcher refuses to edit textually (quoted key, anchors, aliases, merge keys).
fn find_key_span(lines: &[&str], key: &str) -> Result<Option<(usize, usize)>> {
    let prefix = format!("{}:", key);
    let mut start = None;
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with(['"', '\''])
            || line.starts_with("<<")
            || line.contains(": &")
            || line.contains(": *")
        {
            anyhow::bail!("frontmatter shape needs the structural rewrite path");
        }
        if start.is_none() && line.starts_with(&prefix) {
            start = Some(i);
        }
    }
    let Some(start) = start else {
        return Ok(None);
    };
    let mut end = start + 1;
    while end < lines.len() {
        let line = lines[end];
        let continuation =
            line.starts_with([' ', '\t']) || line.starts_with("- ") || line.trim().is_empty();
        if !continuation {
            break;
        }
        end += 1;
    }
    while end > start + 1 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    Ok(Some((start, end)))
}

/// YAML text of a scalar value; collections are refused (structural path).
fn render_scalar(value: &Value) -> Result<String> {
    match value {
        Value::Array(_) | Value::Object(_) => {
            anyhow::bail!("collection values need the structural rewrite path")
        }
        other => Ok(serde_yaml::to_string(other)?.trim_end().to_string()),
    }
}

/// Replaces (or appends) top-level scalar keys in the raw frontmatter text,
/// leaving every other byte of the note untouched.
pub fn patch_frontmatter_keys(raw: &str, edits: &[(&str, Value)]) -> Result<String> {
    let after_open = opening_fence(raw).context("Malformed frontmatter: no opening '---' fence")?;
    let (yaml_end, _) = closing_fence(after_open)
        .context("Malformed frontmatter: opening '---' fence is never closed")?;
    let eol = if raw.contains("\r\n") { "\r\n" } else { "\n" };

    let mut yaml = after_open[..yaml_end].to_string();
    for (key, value) in edits {
        let rendered = render_scalar(value)?.replace('\n', eol);
        let lines: Vec<&str> = yaml.split_inclusive('\n').collect();
        yaml = match find_key_span(&lines, key)? {
            Some((start, end)) => {
                let mut out: String = lines[..start].concat();
                out.push_str(&format!("{}: {}{}", key, rendered, eol));
                out.push_str(&lines[end..].concat());
                out
            }
            None => {
                let sep = if yaml.is_empty() || yaml.ends_with('\n') {
                    ""
                } else {
                    eol
                };
                format!("{}{}{}: {}{}", yaml, sep, key, rendered, eol)
            }
        };
    }

    let open_len = raw.len() - after_open.len();
    Ok(format!(
        "{}{}{}",
        &raw[..open_len],
        yaml,
        &after_open[yaml_end..]
    ))
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
    let count = ATOMIC_WRITE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let rand_id = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let tmp_path = target.with_extension(format!("tmp.{}.{}.{}", pid, count, rand_id));

    {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options
            .open(&tmp_path)
            .with_context(|| format!("Failed to create tmp file: {}", tmp_path.display()))?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
    }

    if target.exists() {
        let perms = match fs::metadata(target) {
            Ok(m) => m.permissions(),
            Err(e) => {
                let _ = fs::remove_file(&tmp_path);
                return Err(e.into());
            }
        };
        if let Err(e) = fs::set_permissions(&tmp_path, perms) {
            let _ = fs::remove_file(&tmp_path);
            return Err(e.into());
        }
    }

    fs::rename(&tmp_path, target).with_context(|| {
        let _ = fs::remove_file(&tmp_path);
        format!(
            "Failed to atomically rename tmp file to {}",
            target.display()
        )
    })?;

    Ok(())
}

/// First note matching `query`, in a deterministic order (see `resolve_all`).
pub fn resolve_note_file(vault: &Path, query: &str) -> Option<PathBuf> {
    resolve_all(vault, query).into_iter().next()
}

/// The single note matching `query`; an ambiguous or missing match is an error
/// so a mutation can never land on an arbitrary one of several same-named notes.
pub fn resolve_note_file_unique(vault: &Path, query: &str) -> Result<PathBuf> {
    let mut found = resolve_all(vault, query);
    match found.len() {
        0 => anyhow::bail!("Note not found: {}", query),
        1 => Ok(found.remove(0)),
        _ => {
            let listed: Vec<String> = found
                .iter()
                .map(|p| {
                    p.strip_prefix(vault)
                        .unwrap_or(p)
                        .to_string_lossy()
                        .into_owned()
                })
                .collect();
            anyhow::bail!("Ambiguous note '{}': matches {}", query, listed.join(", "))
        }
    }
}

/// Every note matching `query`. An exact vault-relative path wins outright;
/// otherwise stem/title matches are returned exact-stem first, then by rel_path.
fn resolve_all(vault: &Path, query: &str) -> Vec<PathBuf> {
    let q = query.trim().trim_matches(|c| c == '\'' || c == '"');
    if q.is_empty() || q.starts_with('/') || q.starts_with('~') {
        return Vec::new();
    }

    let clean_q = q.strip_suffix(".md").unwrap_or(q);
    let hyphenated = clean_q.replace(' ', "-");
    let spaced = clean_q.replace('-', " ");

    // 1-2. Exact path, then normalized space <-> hyphen path candidates.
    let direct = [
        q.to_string(),
        format!("{}.md", clean_q),
        format!("{}.md", hyphenated),
    ];
    for rel in &direct {
        if let Some(p) = contained_path(vault, rel).filter(|p| p.is_file()) {
            return vec![p];
        }
    }

    let from_cache = cache_candidates(vault, clean_q, &hyphenated, &spaced);
    if !from_cache.is_empty() {
        return from_cache;
    }
    walk_candidates(vault, clean_q, &hyphenated, &spaced)
}

/// Cache lookup: exact-stem rows first, then rel_path order; rows whose file is
/// gone or whose path is not contained in the vault are dropped.
fn cache_candidates(vault: &Path, clean_q: &str, hyphenated: &str, spaced: &str) -> Vec<PathBuf> {
    let cache_db = vault.join(".akatsuki/cache.db");
    if !cache_db.is_file() {
        return Vec::new();
    }
    let Ok(con) = rusqlite::Connection::open_with_flags(
        &cache_db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    ) else {
        return Vec::new();
    };
    let sql = "SELECT rel_path FROM entities \
               WHERE stem = ?1 COLLATE NOCASE OR stem = ?2 COLLATE NOCASE \
               OR title = ?1 COLLATE NOCASE OR title = ?3 COLLATE NOCASE \
               ORDER BY (stem = ?1 COLLATE NOCASE) DESC, rel_path";
    let Ok(mut stmt) = con.prepare(sql) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map(rusqlite::params![clean_q, hyphenated, spaced], |row| {
        row.get::<_, String>(0)
    }) else {
        return Vec::new();
    };
    rows.flatten()
        .filter_map(|rel| contained_path(vault, &rel))
        .filter(|p| p.is_file())
        .collect()
}

/// Filesystem fallback comparing stems. Hidden entries below the vault root are
/// skipped (the root itself may legitimately be a dot-directory); matches are
/// sorted so the result never depends on readdir order.
fn walk_candidates(vault: &Path, clean_q: &str, hyphenated: &str, spaced: &str) -> Vec<PathBuf> {
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
        let hidden = e.depth() > 0 && name.starts_with('.');
        !hidden && !ignored_dirs.iter().any(|ig| *ig == name)
    });

    let q_stem_lower = clean_q.to_lowercase();
    let q_hyphen_lower = hyphenated.to_lowercase();
    let q_space_lower = spaced.to_lowercase();

    let mut found: Vec<PathBuf> = walker
        .flatten()
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .filter(|path| {
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                return false;
            };
            let stem_lower = stem.to_lowercase();
            let stem_space = stem_lower.replace('-', " ");
            stem_lower == q_stem_lower
                || stem_lower == q_hyphen_lower
                || stem_space == q_space_lower
                || stem_space == q_stem_lower
        })
        .collect();
    found.sort();
    found
}

pub fn normalize_heading(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Level and normalized text of a markdown heading line. A line indented by
/// four or more columns is an indented code block, never a heading.
pub fn heading_at(line: &str) -> Option<(usize, String)> {
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    if line[..indent].contains('\t') || indent >= 4 {
        return None;
    }
    let trimmed = line.trim();
    if !trimmed.starts_with('#') {
        return None;
    }
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    let rest = &trimmed[level..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    Some((level, normalize_heading(rest)))
}

/// Fence marker character and run length when `line` opens a fenced code block.
fn fence_open(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start();
    let marker = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let run = trimmed.chars().take_while(|c| *c == marker).count();
    (run >= 3).then_some((marker, run))
}

/// True when `line` closes a fence opened with `marker` repeated `open_len` times.
fn fence_closes(line: &str, marker: char, open_len: usize) -> bool {
    let trimmed = line.trim();
    let run = trimmed.chars().take_while(|c| *c == marker).count();
    run >= open_len && trimmed[run..].is_empty()
}

/// First heading accepted by `predicate`, ending at the next heading of the same
/// or higher level; headings inside fenced code blocks are ignored.
fn scan_section(lines: &[&str], predicate: &dyn Fn(&str) -> bool) -> Option<(usize, usize, usize)> {
    let mut in_fence: Option<(char, usize)> = None;
    let mut found: Option<(usize, usize)> = None;
    for (index, line) in lines.iter().enumerate() {
        if let Some((marker, open_len)) = in_fence {
            if fence_closes(line, marker, open_len) {
                in_fence = None;
            }
            continue;
        }
        if let Some(opened) = fence_open(line) {
            in_fence = Some(opened);
            continue;
        }
        let Some((level, name)) = heading_at(line) else {
            continue;
        };
        match found {
            None if predicate(&name) => found = Some((index, level)),
            Some((start, start_level)) if level <= start_level => {
                return Some((start, start_level, index));
            }
            _ => {}
        }
    }
    found.map(|(start, level)| (start, level, lines.len()))
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
    scan_section(lines, &|name| name == target)
        .or_else(|| scan_section(lines, &|name| name.contains(&target)))
}

/// Exact-match variant for writers: a typo or short heading must never resolve
/// to an unrelated section that merely contains the text.
pub fn locate_section_exact(lines: &[&str], heading: &str) -> Option<(usize, usize, usize)> {
    let target = normalize_heading(heading);
    if target.is_empty() {
        return None;
    }
    scan_section(lines, &|name| name == target)
}

pub fn extract_section(content: &str, section_name: &str) -> Option<String> {
    let lines: Vec<&str> = body_after_frontmatter(content).lines().collect();
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
            .and_then(|v| match v {
                Value::String(s) => Some(s.trim().to_string()),
                Value::Number(n) => Some(n.to_string()),
                Value::Bool(b) => Some(b.to_string()),
                _ => None,
            })
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
    let char_budget = budget.saturating_mul(4);
    if text.len() <= char_budget {
        return text.to_string();
    }

    let mut packed = String::new();
    let mut used = 0usize;
    let mut in_frontmatter = opening_fence(text).is_some();
    let mut fences = 0usize;
    let mut frontmatter_chars = 0usize;

    for line in text.split_inclusive('\n') {
        if in_frontmatter {
            packed.push_str(line);
            used += line.len();
            frontmatter_chars += line.len();
            if is_fence(line) {
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
