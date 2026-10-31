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
    locate_section_exact, machine_id, parse_frontmatter, patch_frontmatter_keys, resolve_note_file,
    resolve_note_file_unique, split_frontmatter, write_atomic, VaultLock,
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
                bail!(
                    "Cannot mutate internal akatsuki metadata files: {}",
                    rel_path
                );
            }
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
            for comp in rel.components() {
                if let std::path::Component::Normal(c) = comp {
                    let s = c.to_string_lossy();
                    if s == ".akatsuki" || s == ".akatsuki.lock" || s.starts_with(".akatsuki") {
                        bail!(
                            "Cannot mutate internal akatsuki metadata files: {}",
                            rel_path
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

/// Rejects line breaks and other control characters in single-line fields (paths,
/// headings, keypaths, log text): they would forge extra bullets, headings or
/// audit lines in the notes these fields are written into.
fn reject_control_chars(field: &str, value: &str) -> Result<()> {
    if value.chars().any(|c| c.is_control() && c != '\t') {
        bail!(
            "{} must not contain line breaks or control characters",
            field
        );
    }
    Ok(())
}

/// Re-applies CRLF line endings when `original` used them, so a one-line edit does
/// not turn into a whole-file diff.
fn restore_eol(original: &str, out: String) -> String {
    if original.contains("\r\n") {
        out.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        out
    }
}

pub fn write_note(
    vault: &Path,
    rel_path: &str,
    content: &str,
    overwrite: bool,
    raw: bool,
) -> Result<PathBuf> {
    reject_control_chars("Note path", rel_path)?;
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
    sync_index(vault);

    Ok(target_path)
}

pub fn append_work_log(
    vault: &Path,
    project: &str,
    summary: &str,
    device: Option<&str>,
) -> Result<String> {
    reject_control_chars("Project", project)?;
    reject_control_chars("Summary", summary)?;
    if let Some(device) = device {
        reject_control_chars("Device", device)?;
    }
    let _lock = VaultLock::acquire(vault)?;
    let entry = append_work_log_inner(vault, project, summary, device)?;
    sync_index(vault);
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
    // the legacy ledger layout. Isolate the markdown body from YAML frontmatter
    // before heading lookup so `#` comments inside frontmatter cannot hijack the splice.
    let target_heading = "## 📝 Work Log & Session Notes";
    let (prefix, body) = split_frontmatter(&content).unwrap_or(("", content.as_str()));
    let lines: Vec<&str> = body.lines().collect();
    let new_body = match locate_section(&lines, target_heading) {
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
            body.trim_end(),
            target_heading,
            entry
        ),
    };
    let new_content = restore_eol(&content, format!("{}{}", prefix, new_body));

    write_atomic(&daily_file, &new_content)?;
    Ok(entry)
}

pub fn replace_section_in_note(
    vault: &Path,
    rel_path: &str,
    heading: &str,
    new_section_content: &str,
) -> Result<()> {
    reject_control_chars("Note path", rel_path)?;
    reject_control_chars("Heading", heading)?;
    ensure_writable_note_path(vault, rel_path, None)?;
    let _lock = VaultLock::acquire(vault)?;

    let note_path = resolve_note_file_unique(vault, rel_path)?;
    ensure_writable_note_path(vault, rel_path, Some(&note_path))?;

    let text = fs::read_to_string(&note_path)?;
    let (mut fm, body) = parse_frontmatter(&text)
        .with_context(|| format!("Note '{}' has invalid frontmatter", note_path.display()))?;

    stamp_provenance(&mut fm);
    let edit = |b: &str| {
        replaced_body(b, heading, new_section_content).with_context(|| {
            format!(
                "Heading '{}' not found in note '{}' (writers require an exact heading match)",
                heading,
                note_path.display()
            )
        })
    };
    let out = rewrite_note(&text, &fm, &body, Some(Vec::new()), &edit)?;
    write_atomic(&note_path, &out)?;
    record_audit(vault, "replace", &format!("{} #{}", rel_path, heading))?;
    sync_index(vault);

    Ok(())
}

/// Sets a keypath from a CLI/MCP string, typing it through `parse_scalar`.
pub fn set_note_property(vault: &Path, rel_path: &str, keypath: &str, value: &str) -> Result<()> {
    set_note_value(vault, rel_path, keypath, parse_scalar(value))
}

/// Sets a keypath to an already-typed value (no string re-interpretation).
pub fn set_note_value(vault: &Path, rel_path: &str, keypath: &str, value: Value) -> Result<()> {
    reject_control_chars("Note path", rel_path)?;
    reject_control_chars("Keypath", keypath)?;
    ensure_writable_note_path(vault, rel_path, None)?;
    let _lock = VaultLock::acquire(vault)?;

    let note_path = resolve_note_file_unique(vault, rel_path)?;
    ensure_writable_note_path(vault, rel_path, Some(&note_path))?;

    let text = fs::read_to_string(&note_path)?;
    let (mut fm, body) = parse_frontmatter(&text)
        .with_context(|| format!("Note '{}' has invalid frontmatter", note_path.display()))?;

    let top_level = (!keypath.trim().contains('.')).then(|| vec![(keypath.trim(), value.clone())]);
    set_keypath(&mut fm, keypath, value)?;
    stamp_provenance(&mut fm);

    let out = rewrite_note(&text, &fm, &body, top_level, &|b| Ok(b.to_string()))?;
    write_atomic(&note_path, &out)?;
    record_audit(vault, "set", &format!("{} {}", rel_path, keypath))?;
    sync_index(vault);

    Ok(())
}

pub fn append_section_in_note(
    vault: &Path,
    rel_path: &str,
    heading: &str,
    content_to_append: &str,
) -> Result<()> {
    reject_control_chars("Note path", rel_path)?;
    reject_control_chars("Heading", heading)?;
    ensure_writable_note_path(vault, rel_path, None)?;
    let _lock = VaultLock::acquire(vault)?;

    // A missing note is created; an ambiguous one is an error, never a guess.
    let note_path = if resolve_note_file(vault, rel_path).is_some() {
        let path = resolve_note_file_unique(vault, rel_path)?;
        ensure_writable_note_path(vault, rel_path, Some(&path))?;
        path
    } else {
        create_note(vault, rel_path, heading)?
    };

    let text = fs::read_to_string(&note_path)?;
    let (mut fm, body) = parse_frontmatter(&text)
        .with_context(|| format!("Note '{}' has invalid frontmatter", note_path.display()))?;

    stamp_provenance(&mut fm);
    let edit = |b: &str| Ok(appended_body(b, heading, content_to_append));
    let out = rewrite_note(&text, &fm, &body, Some(Vec::new()), &edit)?;
    write_atomic(&note_path, &out)?;
    record_audit(vault, "append", &format!("{} #{}", rel_path, heading))?;
    sync_index(vault);

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

/// New body with `heading`'s section replaced by `new_section_content`; `None`
/// when no section matches the heading exactly.
fn replaced_body(body: &str, heading: &str, new_section_content: &str) -> Option<String> {
    let lines: Vec<&str> = body.lines().collect();
    let (start, _level, end) = locate_section_exact(&lines, heading)?;

    let replacement = new_section_content.trim();
    let first_line = replacement.lines().next().unwrap_or("");
    let replacement_has_heading = crate::storage::heading_at(first_line)
        .map(|(_, name)| name == crate::storage::normalize_heading(heading))
        .unwrap_or(false);

    let mut rebuilt: Vec<String> = if replacement_has_heading {
        lines[..start].iter().map(|l| l.to_string()).collect()
    } else {
        lines[..=start].iter().map(|l| l.to_string()).collect()
    };

    if !replacement.is_empty() {
        if !rebuilt.is_empty() && !replacement_has_heading {
            rebuilt.push(String::new());
        }
        rebuilt.extend(replacement.lines().map(|l| l.to_string()));
    }
    if end < lines.len() {
        rebuilt.push(String::new());
        rebuilt.extend(lines[end..].iter().map(|l| l.to_string()));
    }
    Some(rebuilt.join("\n"))
}

/// New body with `content_to_append` added to `heading`'s section; a missing
/// heading is created at the end of the note.
fn appended_body(body: &str, heading: &str, content_to_append: &str) -> String {
    let appended = content_to_append.trim_end();
    let lines: Vec<&str> = body.lines().collect();
    match locate_section_exact(&lines, heading) {
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
        None => format!(
            "{}\n\n{}\n\n{}\n",
            body.trim_end(),
            normalize_section_heading(heading),
            appended
        ),
    }
}

/// The `updated` / `updated_by` provenance keys as currently stamped in `fm`.
fn provenance_edits(fm: &Value) -> Vec<(&'static str, Value)> {
    ["updated", "updated_by"]
        .into_iter()
        .filter_map(|key| fm.get(key).map(|v| (key, v.clone())))
        .collect()
}

/// Assembles the note to write. When the note has a well-formed frontmatter block
/// and `extra` is `Some`, only the touched top-level scalar keys (`extra` plus the
/// provenance stamp) are patched textually, so comments, `1.10`, `0x1F` and blank
/// lines survive and the body is edited from its raw bytes. `None` (nested
/// keypaths) or a shape the patcher refuses falls back to the structural
/// parse/dump roundtrip.
fn rewrite_note(
    text: &str,
    fm: &Value,
    parsed_body: &str,
    extra: Option<Vec<(&str, Value)>>,
    edit: &dyn Fn(&str) -> Result<String>,
) -> Result<String> {
    if let (Some(mut edits), Some((_, raw_body))) = (extra, split_frontmatter(text)) {
        edits.extend(provenance_edits(fm));
        if let Ok(patched) = patch_frontmatter_keys(text, &edits) {
            if let Some((block, _)) = split_frontmatter(&patched) {
                return Ok(restore_eol(text, format!("{}{}", block, edit(raw_body)?)));
            }
        }
    }
    Ok(restore_eol(text, dump_frontmatter(fm, &edit(parsed_body)?)))
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

/// Refreshes the projection after a mutation has already been committed to disk.
/// Non-fatal on purpose: the note and its audit line are durable, so reporting an
/// error would make a retrying caller duplicate the write; every read path
/// re-syncs on open.
fn sync_index(vault: &Path) {
    let synced = open_cache_db(vault).and_then(|mut con| sync_vault_index(vault, &mut con));
    if let Err(e) = synced {
        eprintln!("⚠ index sync failed after a successful write (retried on next read): {e:#}");
    }
}

/// Walks (creating intermediate maps) a dotted keypath and stores `value` at its end.
fn set_keypath(fm: &mut Value, keypath: &str, value: Value) -> Result<()> {
    let trimmed = keypath.trim();
    if trimmed.is_empty() {
        bail!("Empty keypath");
    }

    if let Some(map) = fm.as_object_mut() {
        if map.contains_key(trimmed) {
            map.insert(trimmed.to_string(), value);
            return Ok(());
        }
    }

    let segments: Vec<&str> = trimmed
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

/// Interprets a CLI/MCP string as the YAML scalar it looks like. Numbers are
/// typed only in their canonical spelling, so `007`, `1.10` and `+5` stay strings
/// instead of being silently rewritten; a quoted value (`"123"`) is the string
/// inside the quotes.
fn parse_scalar(raw: &str) -> Value {
    let quoted = raw.len() >= 2
        && ((raw.starts_with('"') && raw.ends_with('"'))
            || (raw.starts_with('\'') && raw.ends_with('\'')));
    if quoted {
        return json!(&raw[1..raw.len() - 1]);
    }
    if let Ok(number) = raw.parse::<i64>() {
        if number.to_string() == raw {
            return json!(number);
        }
    }
    if let Ok(float) = raw.parse::<f64>() {
        if float.is_finite()
            && raw.contains('.')
            && serde_json::Number::from_f64(float).is_some_and(|n| n.to_string() == raw)
        {
            return json!(float);
        }
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
        if let Ok(parsed) = serde_yaml::from_str::<Value>(raw) {
            return parsed;
        }
    }
    json!(raw)
}
