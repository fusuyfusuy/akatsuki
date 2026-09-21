//! Vault mutations: Thread-safe and process-safe atomic note write, section replace, and work log append.

use std::fs;
use std::path::{Path, PathBuf};
use anyhow::{bail, Context, Result};
use chrono::Local;
use serde_json::Value;

use crate::index::{open_cache_db, sync_vault_index};
use crate::storage::{contained_path, dump_frontmatter, is_raw_path, parse_frontmatter, resolve_note_file, write_atomic, VaultLock};

pub fn write_note(
    vault: &Path,
    rel_path: &str,
    content: &str,
    overwrite: bool,
    raw: bool,
) -> Result<PathBuf> {
    let _lock = VaultLock::acquire(vault)?;

    let clean = rel_path.trim().trim_start_matches('/');
    let target_rel = if !raw && !is_raw_path(clean) && !clean.ends_with(".md") {
        format!("{}.md", clean)
    } else {
        clean.to_string()
    };

    let target_path = contained_path(vault, &target_rel)
        .context("Path traversal outside vault boundary is forbidden")?;

    if target_path.exists() && !overwrite {
        bail!("File '{}' already exists. Pass overwrite=true to replace.", target_rel);
    }

    write_atomic(&target_path, content)?;

    let mut con = open_cache_db(vault)?;
    let _ = sync_vault_index(vault, &mut con)?;

    Ok(target_path)
}

pub fn append_work_log(
    vault: &Path,
    project: &str,
    summary: &str,
    device: Option<&str>,
) -> Result<String> {
    let _lock = VaultLock::acquire(vault)?;

    let now = Local::now();
    let date_str = now.format("%Y-%m-%d").to_string();
    let time_str = now.format("%H:%M").to_string();

    let hostname = device
        .map(|s| s.to_string())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "local".to_string());

    let entry = if !project.is_empty() {
        format!("- **{}** [{}]: [{}] {}", time_str, hostname, project, summary)
    } else {
        format!("- **{}** [{}]: {}", time_str, hostname, summary)
    };

    let daily_dir = vault.join("01-Daily");
    fs::create_dir_all(&daily_dir)?;
    let daily_file = daily_dir.join(format!("{}.md", date_str));

    let content = if daily_file.is_file() {
        fs::read_to_string(&daily_file)?
    } else {
        format!(
            "---\ntitle: \"{}\"\ndate: {}\ntype: daily\ntags: [daily, worklog]\nsummary: \"Daily activity log for {}\"\n---\n\n# {}\n\n## 📝 Work Log & Session Notes\n\n",
            date_str, date_str, date_str, date_str
        )
    };

    let target_heading = "## 📝 Work Log & Session Notes";
    let new_content = if let Some(idx) = content.find(target_heading) {
        let after_heading = idx + target_heading.len();
        let mut head = content[..after_heading].to_string();
        let tail = &content[after_heading..];

        head.push('\n');
        head.push_str(&entry);
        head.push('\n');
        head.push_str(tail.trim_start_matches('\n'));
        head
    } else {
        format!("{}\n\n{}\n\n{}\n", content.trim_end(), target_heading, entry)
    };

    write_atomic(&daily_file, &new_content)?;

    let mut con = open_cache_db(vault)?;
    let _ = sync_vault_index(vault, &mut con)?;

    Ok(entry)
}

pub fn replace_section_in_note(
    vault: &Path,
    rel_path: &str,
    heading: &str,
    new_section_content: &str,
) -> Result<()> {
    let _lock = VaultLock::acquire(vault)?;

    let note_path = resolve_note_file(vault, rel_path)
        .with_context(|| format!("Note '{}' not found in vault", rel_path))?;

    let text = fs::read_to_string(&note_path)?;
    let (fm, body) = parse_frontmatter(&text);

    let norm_h = heading.trim().trim_start_matches('#').trim();
    let lines: Vec<&str> = body.lines().collect();

    let mut start_idx = None;
    let mut end_idx = None;
    let mut target_level = 2;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            let h_text = trimmed[level..].trim();
            if h_text.eq_ignore_ascii_case(norm_h) {
                start_idx = Some(i);
                target_level = level;
                continue;
            }
            if start_idx.is_some() && level <= target_level {
                end_idx = Some(i);
                break;
            }
        }
    }

    let start = match start_idx {
        Some(idx) => idx,
        None => bail!("Heading '{}' not found in note '{}'", heading, note_path.display()),
    };
    let end = end_idx.unwrap_or(lines.len());

    let mut new_body_lines = Vec::new();
    for line in &lines[..=start] {
        new_body_lines.push(*line);
    }
    new_body_lines.push("");
    new_body_lines.push(new_section_content.trim());
    new_body_lines.push("");
    for line in &lines[end..] {
        new_body_lines.push(*line);
    }

    let updated_body = new_body_lines.join("\n");
    let full_content = dump_frontmatter(&fm, &updated_body);

    write_atomic(&note_path, &full_content)?;

    let mut con = open_cache_db(vault)?;
    let _ = sync_vault_index(vault, &mut con)?;

    Ok(())
}

pub fn set_note_property(
    vault: &Path,
    rel_path: &str,
    keypath: &str,
    value: &str,
) -> Result<()> {
    let _lock = VaultLock::acquire(vault)?;

    let note_path = resolve_note_file(vault, rel_path)
        .with_context(|| format!("Note '{}' not found in vault", rel_path))?;

    let text = fs::read_to_string(&note_path)?;
    let (mut fm, body) = parse_frontmatter(&text);

    if let Value::Object(ref mut map) = fm {
        let val: Value = if let Ok(n) = value.parse::<i64>() {
            Value::Number(n.into())
        } else if let Ok(b) = value.parse::<bool>() {
            Value::Bool(b)
        } else if value.starts_with('[') && value.ends_with(']') {
            serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_string()))
        } else {
            Value::String(value.to_string())
        };

        map.insert(keypath.to_string(), val);
    }

    let updated = dump_frontmatter(&fm, &body);
    write_atomic(&note_path, &updated)?;

    let mut con = open_cache_db(vault)?;
    let _ = sync_vault_index(vault, &mut con)?;

    Ok(())
}
