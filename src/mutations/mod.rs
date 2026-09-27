//! Vault mutations: process-safe atomic note writes, section surgery, and the work log.
//!
//! Every mutation takes the vault lock exactly once, stamps provenance
//! (`updated`, `updated_by`), records an audit line in the daily note, and
//! re-syncs the projection. Locking is not re-entrant: `flock` on a second file
//! descriptor of the same lock file deadlocks within one process, so nested
//! helpers deliberately take no lock.

use anyhow::{bail, Context, Result};
use chrono::{Local, SecondsFormat};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

use crate::index::{open_cache_db, sync_vault_index};
use crate::storage::{
    auto_heal_frontmatter, contained_path, dump_frontmatter, is_raw_path, locate_section,
    machine_id, parse_frontmatter, resolve_note_file, write_atomic, VaultLock,
};

/// The project that owns vault-maintenance entries in the daily audit trail.
const AUDIT_PROJECT: &str = "akatsuki";

/// Asserts that a note path does not target internal engine metadata files (.akatsuki, .akatsuki.lock).
fn ensure_writable_note_path(
    vault: &Path,
    rel_path: &str,
    resolved_path: Option<&Path>,
) -> Result<()> {
    let trimmed = rel_path.trim();
    let p = Path::new(trimmed);
    for comp in p.components() {
        if let std::path::Component::Normal(c) = comp {
            let s = c.to_string_lossy();
            if s == ".akatsuki" || s == ".akatsuki.lock" || s.starts_with(".akatsuki") {
                bail!("Cannot mutate internal akatsuki metadata files: {}", rel_path);
            }
            break;
        }
    }

    if let Some(target) = resolved_path {
        let abs_vault = if vault.is_relative() {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(vault)
        } else {
            vault.to_path_buf()
        };
        let norm_vault = crate::storage::normalize_path(&abs_vault);
        let norm_target = crate::storage::normalize_path(target);
        if let Ok(rel) = norm_target.strip_prefix(&norm_vault) {
            if let Some(std::path::Component::Normal(first)) = rel.components().next() {
                let s = first.to_string_lossy();
                if s == ".akatsuki" || s == ".akatsuki.lock" || s.starts_with(".akatsuki") {
                    bail!("Cannot mutate internal akatsuki metadata files: {}", rel_path);
                }
            }
        }
    }
    Ok(())
}

pub fn write_note(
    vault: &Path,
    rel_path: &str,
    content: &str,
    overwrite: bool,
    raw: bool,
) -> Result<PathBuf> {
    ensure_writable_note_path(vault, rel_path, None)?;
    let _lock = VaultLock::acquire(vault)?;

    let trimmed = rel_path.trim();
    if trimmed.starts_with('/') || trimmed.starts_with('~') {
        bail!(
            "Path '{}' is absolute; note paths must be vault-relative (e.g. '20-Projects/my-app.md')",
            trimmed
        );
    }
    let target_rel = if !raw && !is_raw_path(trimmed) && !trimmed.ends_with(".md") {
        format!("{}.md", trimmed)
    } else {
        trimmed.to_string()
    };

    let target_path = contained_path(vault, &target_rel)
        .context("Path traversal outside vault boundary is forbidden")?;
    ensure_writable_note_path(vault, rel_path, Some(&target_path))?;

    if target_path.exists() && !overwrite {
        bail!(
            "File '{}' already exists. Pass overwrite=true to replace.",
            target_rel
        );
    }

    // Raw artifacts are stored verbatim; markdown notes get their frontmatter
    // completed so the strict lint gate can pass on a freshly written note.
    let payload = if !raw && target_rel.ends_with(".md") {
        auto_heal_frontmatter(content, &target_rel)?
    } else {
        content.to_string()
    };

    write_atomic(&target_path, &payload)?;
    record_audit(vault, "write", &target_rel)?;
    sync_index(vault)?;

    Ok(target_path)
}

pub fn append_work_log(
    vault: &Path,
    project: &str,
    summary: &str,
    device: Option<&str>,
) -> Result<String> {
    let _lock = VaultLock::acquire(vault)?;
    let entry = append_work_log_inner(vault, project, summary, device)?;
    sync_index(vault)?;
    Ok(entry)
}

/// Appends `- **HH:MM** [device]: [project] summary` to today's daily note.
///
/// Lock-free: callers that already hold the vault lock MUST use this directly.
fn append_work_log_inner(
    vault: &Path,
    project: &str,
    summary: &str,
    device: Option<&str>,
) -> Result<String> {
    let now = Local::now();
    let date_str = now.format("%Y-%m-%d").to_string();
    let time_str = now.format("%H:%M").to_string();
    let hostname = device.map(|s| s.to_string()).unwrap_or_else(machine_id);

    let entry = if project.is_empty() {
        format!("- **{}** [{}]: {}", time_str, hostname, summary)
    } else {
        format!(
            "- **{}** [{}]: [{}] {}",
            time_str, hostname, project, summary
        )
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

    // Log lines are chronological: the newest entry ends the section, matching
    // the legacy ledger layout.
    let target_heading = "## 📝 Work Log & Session Notes";
    let lines: Vec<&str> = content.lines().collect();
    let new_content = match locate_section(&lines, target_heading) {
        Some((_start, _level, end)) => {
            let mut rebuilt: Vec<String> = lines[..end].iter().map(|l| l.to_string()).collect();
            while rebuilt.last().is_some_and(|l| l.trim().is_empty()) {
                rebuilt.pop();
            }
            rebuilt.push(entry.clone());
            rebuilt.push(String::new());
            rebuilt.extend(lines[end..].iter().map(|l| l.to_string()));
            rebuilt.join("\n")
        }
        None => format!(
            "{}\n\n{}\n\n{}\n",
            content.trim_end(),
            target_heading,
            entry
        ),
    };

    write_atomic(&daily_file, &new_content)?;
    Ok(entry)
}

pub fn replace_section_in_note(
    vault: &Path,
    rel_path: &str,
    heading: &str,
    new_section_content: &str,
) -> Result<()> {
    ensure_writable_note_path(vault, rel_path, None)?;
    let _lock = VaultLock::acquire(vault)?;

    let note_path = resolve_note_file(vault, rel_path)
        .with_context(|| format!("Note '{}' not found in vault", rel_path))?;
    ensure_writable_note_path(vault, rel_path, Some(&note_path))?;

    let text = fs::read_to_string(&note_path)?;
    let (mut fm, body) = parse_frontmatter(&text)
        .with_context(|| format!("Note '{}' has invalid frontmatter", note_path.display()))?;

    let lines: Vec<&str> = body.lines().collect();
    let (start, _level, end) = locate_section(&lines, heading).with_context(|| {
        format!(
            "Heading '{}' not found in note '{}'",
            heading,
            note_path.display()
        )
    })?;

    let mut rebuilt: Vec<String> = lines[..=start].iter().map(|l| l.to_string()).collect();
    let replacement = new_section_content.trim_end();
    if !replacement.is_empty() {
        rebuilt.push(String::new());
        rebuilt.extend(replacement.lines().map(|l| l.to_string()));
    }
    if end < lines.len() {
        rebuilt.push(String::new());
        rebuilt.extend(lines[end..].iter().map(|l| l.to_string()));
    }

    stamp_provenance(&mut fm);
    write_atomic(&note_path, &dump_frontmatter(&fm, &rebuilt.join("\n")))?;
    record_audit(vault, "replace", &format!("{} #{}", rel_path, heading))?;
    sync_index(vault)?;

    Ok(())
}

pub fn set_note_property(vault: &Path, rel_path: &str, keypath: &str, value: &str) -> Result<()> {
    ensure_writable_note_path(vault, rel_path, None)?;
    let _lock = VaultLock::acquire(vault)?;

    let note_path = resolve_note_file(vault, rel_path)
        .with_context(|| format!("Note '{}' not found in vault", rel_path))?;
    ensure_writable_note_path(vault, rel_path, Some(&note_path))?;

    let text = fs::read_to_string(&note_path)?;
    let (mut fm, body) = parse_frontmatter(&text)
        .with_context(|| format!("Note '{}' has invalid frontmatter", note_path.display()))?;

    set_keypath(&mut fm, keypath, parse_scalar(value))?;
    stamp_provenance(&mut fm);

    write_atomic(&note_path, &dump_frontmatter(&fm, &body))?;
    record_audit(vault, "set", &format!("{} {}", rel_path, keypath))?;
    sync_index(vault)?;

    Ok(())
}

pub fn append_section_in_note(
    vault: &Path,
    rel_path: &str,
    heading: &str,
    content_to_append: &str,
) -> Result<()> {
    ensure_writable_note_path(vault, rel_path, None)?;
    let _lock = VaultLock::acquire(vault)?;

    let note_path = match resolve_note_file(vault, rel_path) {
        Some(path) => {
            ensure_writable_note_path(vault, rel_path, Some(&path))?;
            path
        }
        None => create_note(vault, rel_path, heading)?,
    };

    let text = fs::read_to_string(&note_path)?;
    let (mut fm, body) = parse_frontmatter(&text)
        .with_context(|| format!("Note '{}' has invalid frontmatter", note_path.display()))?;

    let appended = content_to_append.trim_end();
    let lines: Vec<&str> = body.lines().collect();

    let updated_body = match locate_section(&lines, heading) {
        Some((_start, _level, end)) => {
            let mut rebuilt: Vec<String> = lines[..end].iter().map(|l| l.to_string()).collect();
            while rebuilt.last().is_some_and(|l| l.trim().is_empty()) {
                rebuilt.pop();
            }
            rebuilt.push(String::new());
            rebuilt.extend(appended.lines().map(|l| l.to_string()));
            rebuilt.push(String::new());
            rebuilt.extend(lines[end..].iter().map(|l| l.to_string()));
            rebuilt.join("\n")
        }
        // Parity: a missing heading is created at the end of the note.
        None => format!(
            "{}\n\n{}\n\n{}\n",
            body.trim_end(),
            normalize_section_heading(heading),
            appended
        ),
    };

    stamp_provenance(&mut fm);
    write_atomic(&note_path, &dump_frontmatter(&fm, &updated_body))?;
    record_audit(vault, "append", &format!("{} #{}", rel_path, heading))?;
    sync_index(vault)?;

    Ok(())
}

pub fn read_daily_note(vault: &Path, date_opt: Option<&str>) -> Result<(String, bool)> {
    let date_str = match date_opt {
        Some(d) if !d.trim().is_empty() => {
            let d = d.trim();
            let well_formed = d.len() == 10
                && d.as_bytes()[4] == b'-'
                && d.as_bytes()[7] == b'-'
                && d.bytes().enumerate().all(|(i, b)| {
                    if i == 4 || i == 7 {
                        b == b'-'
                    } else {
                        b.is_ascii_digit()
                    }
                });
            if !well_formed {
                bail!("Invalid date '{}': expected YYYY-MM-DD", d);
            }
            d.to_string()
        }
        _ => Local::now().format("%Y-%m-%d").to_string(),
    };

    let rel = format!("01-Daily/{}.md", date_str);
    let daily_file = contained_path(vault, &rel)
        .with_context(|| format!("Daily note path '{}' escapes the vault", rel))?;

    if daily_file.is_file() {
        let content = fs::read_to_string(&daily_file)?;
        Ok((content, true))
    } else {
        Ok((
            format!("Daily note for {} does not exist yet.", date_str),
            false,
        ))
    }
}

/// Creates a note that does not exist yet, seeded with its frontmatter and the
/// requested heading. Caller holds the vault lock.
fn create_note(vault: &Path, rel_path: &str, heading: &str) -> Result<PathBuf> {
    ensure_writable_note_path(vault, rel_path, None)?;
    let rel = if rel_path.trim().ends_with(".md") {
        rel_path.trim().to_string()
    } else {
        format!("{}.md", rel_path.trim())
    };
    let target =
        contained_path(vault, &rel).with_context(|| format!("Path '{}' escapes the vault", rel))?;
    ensure_writable_note_path(vault, rel_path, Some(&target))?;

    let seed = format!(
        "# {}\n\n{}\n",
        rel_stem(&rel),
        normalize_section_heading(heading)
    );
    let healed = auto_heal_frontmatter(&seed, &rel)?;
    write_atomic(&target, &healed)?;

    Ok(target)
}

/// `## Heading` derived from a caller-supplied heading of any depth.
fn normalize_section_heading(heading: &str) -> String {
    let text = heading.trim().trim_start_matches('#').trim();
    format!("## {}", text)
}

fn rel_stem(rel_path: &str) -> String {
    Path::new(rel_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(rel_path)
        .replace('-', " ")
}

/// Stamps the mutation into the note's frontmatter.
fn stamp_provenance(fm: &mut Value) {
    if let Value::Object(map) = fm {
        map.insert(
            "updated".to_string(),
            json!(Local::now().to_rfc3339_opts(SecondsFormat::Secs, true)),
        );
        map.insert("updated_by".to_string(), json!(machine_id()));
    }
}

/// Writes the mutation ledger line. Caller holds the vault lock.
fn record_audit(vault: &Path, action: &str, target: &str) -> Result<()> {
    append_work_log_inner(
        vault,
        AUDIT_PROJECT,
        &format!("{} {} -> exit 0", action, target),
        None,
    )?;
    Ok(())
}

fn sync_index(vault: &Path) -> Result<()> {
    let mut con = open_cache_db(vault)?;
    sync_vault_index(vault, &mut con)?;
    Ok(())
}

/// Walks (creating intermediate maps) a dotted keypath and stores `value` at its end.
fn set_keypath(fm: &mut Value, keypath: &str, value: Value) -> Result<()> {
    let segments: Vec<&str> = keypath
        .split('.')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if segments.is_empty() {
        bail!("Empty keypath");
    }

    let mut cursor = fm;
    for segment in &segments[..segments.len() - 1] {
        if !cursor.is_object() {
            bail!("Cannot descend into '{}': parent is not a mapping", segment);
        }
        let map = cursor.as_object_mut().expect("checked object");
        cursor = map
            .entry((*segment).to_string())
            .or_insert_with(|| json!({}));
    }

    match cursor.as_object_mut() {
        Some(map) => {
            map.insert(segments[segments.len() - 1].to_string(), value);
            Ok(())
        }
        None => bail!("Cannot set '{}': frontmatter is not a mapping", keypath),
    }
}

/// Interprets a CLI/MCP string as the YAML scalar it looks like.
fn parse_scalar(raw: &str) -> Value {
    if let Ok(number) = raw.parse::<i64>() {
        return json!(number);
    }
    if let Ok(flag) = raw.parse::<bool>() {
        return json!(flag);
    }
    let looks_structured = (raw.starts_with('[') && raw.ends_with(']'))
        || (raw.starts_with('{') && raw.ends_with('}'));
    if looks_structured {
        if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
            return parsed;
        }
    }
    json!(raw)
}
