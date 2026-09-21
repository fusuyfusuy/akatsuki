//! Storage substrate: Vault discovery, path containment, atomic I/O, frontmatter, and locking.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use fs2::FileExt;
use serde_json::Value;

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

        file.lock_exclusive()
            .with_context(|| format!("Failed to acquire exclusive lock on {}", lock_path.display()))?;

        Ok(Self { _file: file })
    }
}

pub fn is_raw_path(rel_path: &str) -> bool {
    let lower = rel_path.to_lowercase();
    if lower == "dockerfile" || lower == "caddyfile" || lower == "makefile" || lower.ends_with(".example") {
        return true;
    }
    RAW_EXTS.iter().any(|ext| lower.ends_with(ext))
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
                    if let Some(val) = trim.strip_prefix("export AKATSUKI_VAULT=")
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
            if dir.join("akatsuki/40-Systems").is_dir() && dir.join("akatsuki/20-Projects").is_dir() {
                let ak = dir.join("akatsuki");
                return ak.canonicalize().unwrap_or(ak);
            }
            if dir.join(".akatsuki").is_dir() || (dir.join("INDEX.md").is_file() && dir.join("AGENTS.md").is_file()) {
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
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return format!("{}/{}", home.display(), rest);
        }
    }
    s.to_string()
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

    // Disallow hidden directory traversals (e.g. .git/hooks)
    for comp in norm_candidate.strip_prefix(&norm_vault).ok()?.components() {
        if let std::path::Component::Normal(c) = comp {
            if let Some(s) = c.to_str() {
                if s.starts_with('.') && s != ".akatsuki" && s != ".akatsuki.lock" {
                    return None;
                }
            }
        }
    }

    Some(norm_candidate)
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

pub fn parse_frontmatter(content: &str) -> (Value, String) {
    if !content.starts_with("---") {
        return (serde_json::json!({}), content.to_string());
    }

    let parts: Vec<&str> = content.splitn(3, "---").collect();
    if parts.len() < 3 {
        return (serde_json::json!({}), content.to_string());
    }

    let raw_yaml = parts[1];
    let body = parts[2].trim_start_matches('\n').to_string();

    match serde_yaml::from_str::<Value>(raw_yaml) {
        Ok(v) => (v, body),
        Err(_) => (serde_json::json!({}), content.to_string()),
    }
}

pub fn dump_frontmatter(meta: &Value, body: &str) -> String {
    if meta.as_object().map_or(true, |m| m.is_empty()) {
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
        format!("Failed to atomically rename tmp file to {}", target.display())
    })?;

    Ok(())
}

pub fn resolve_note_file(vault: &Path, query: &str) -> Option<PathBuf> {
    let q = query.trim().trim_matches(|c| c == '\'' || c == '"');
    if q.is_empty() || q.starts_with('/') || q.starts_with('~') {
        return None;
    }

    let clean_q = if q.ends_with(".md") { &q[..q.len() - 3] } else { q };

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
                if let Ok(rel) = stmt.query_row(rusqlite::params![clean_q, hyphenated, spaced], |row| {
                    row.get::<_, String>(0)
                }) {
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
    let ignored_dirs = [".git", ".akatsuki", ".venv", "node_modules", ".obsidian", "__pycache__", "_templates"];
    let walker = walkdir::WalkDir::new(vault)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !ignored_dirs.iter().any(|ig| *ig == name)
        });

    let q_stem_lower = clean_q.to_lowercase();
    let q_hyphen_lower = hyphenated.to_lowercase();
    let q_space_lower = spaced.to_lowercase();

    for entry in walker.flatten() {
        if entry.file_type().is_file() {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "md") {
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

pub fn extract_section(content: &str, section_name: &str) -> Option<String> {
    let target_norm = normalize_heading(section_name);
    if target_norm.is_empty() {
        return None;
    }
    let lines: Vec<&str> = content.lines().collect();
    let mut capture = false;
    let mut sec_lines = Vec::new();
    let mut cur_level = 2;

    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            let lvl = trimmed.chars().take_while(|c| *c == '#').count();
            let h_name = normalize_heading(&trimmed[lvl..]);
            if h_name == target_norm || h_name.contains(&target_norm) {
                capture = true;
                cur_level = lvl;
                sec_lines.push(line);
                continue;
            }
            if capture && lvl <= cur_level {
                break;
            }
        }
        if capture {
            sec_lines.push(line);
        }
    }

    if !sec_lines.is_empty() {
        Some(sec_lines.join("\n"))
    } else {
        None
    }
}

