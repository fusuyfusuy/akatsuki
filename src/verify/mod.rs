//! Verification, linting, wikilink audit, machine invariant execution, and vault heals.

use anyhow::{bail, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use walkdir::WalkDir;

use crate::constants::{DOMAIN_MOCS, ROOT_ANCHORS};
use crate::index::{
    open_cache_db, open_synced_db, sync_vault_index, sync_vault_index_dry_run, SyncReport,
};
use crate::storage::{parse_frontmatter, write_atomic, VaultLock};

/// Default hard ceiling for a single embedded invariant assertion.
const DEFAULT_INVARIANT_TIMEOUT_SECS: u64 = 10;

/// `AKATSUKI_INVARIANT_TIMEOUT` overrides the ceiling in seconds, for slow hosts
/// and for the regression suite.
fn invariant_timeout() -> Duration {
    std::env::var("AKATSUKI_INVARIANT_TIMEOUT")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(DEFAULT_INVARIANT_TIMEOUT_SECS))
}

/// Markdown notes that participate in the link graph: every `.md` under the vault
/// except `_templates/` and hidden top-level entries, in a stable order.
fn linkable_notes(vault: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut notes = Vec::new();
    for entry in WalkDir::new(vault) {
        let entry = entry.with_context(|| format!("Failed to walk {}", vault.display()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.path().extension().is_none_or(|ext| ext != "md") {
            continue;
        }
        let rel = relative_slash_path(vault, entry.path());
        if rel.starts_with("_templates") || rel.starts_with('.') {
            continue;
        }
        notes.push((rel, entry.path().to_path_buf()));
    }
    notes.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(notes)
}

/// Vault-relative path rendered with `/` separators regardless of platform.
fn relative_slash_path(vault: &Path, path: &Path) -> String {
    path.strip_prefix(vault)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Lowercased file stem, the key a wikilink target resolves against.
fn file_stem_lower(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| path.to_lowercase())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LintReport {
    pub passed: bool,
    pub total_notes: usize,
    pub errors: Vec<String>,
}

/// Python truthiness for a frontmatter field: an empty string, an empty collection,
/// `null`, `false` and `0` all count as absent for a required-field check.
fn field_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(map)) => !map.is_empty(),
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
    }
}

/// Raw (non-markdown) assets whose syntax is linted: everything under `50-Configs/`
/// and `60-Scripts/` except dotfiles, in a stable order.
fn raw_asset_files(vault: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut assets = Vec::new();
    for dir in ["50-Configs", "60-Scripts"] {
        let root = vault.join(dir);
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(&root) {
            let entry = entry.with_context(|| format!("Failed to walk {}", root.display()))?;
            if !entry.file_type().is_file() {
                continue;
            }
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            assets.push((
                relative_slash_path(vault, entry.path()),
                entry.path().to_path_buf(),
            ));
        }
    }
    assets.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(assets)
}

/// Reads a raw asset for linting; an unreadable asset is itself a lint violation.
fn read_asset(rel: &str, abs: &Path) -> std::result::Result<String, String> {
    fs::read_to_string(abs).map_err(|e| format!("'{rel}': File read error during lint: {e}"))
}

pub fn lint_vault(vault: &Path) -> Result<LintReport> {
    let con = open_synced_db(vault)?;

    let repo_test_re = Regex::new(
        r"(?i)(pytest|npm test|cargo test|bun test|python3\s+[\w_-]*test[\w_-]*\.py|python3\s+verify_backup\.py)",
    )?;
    let verify_re = Regex::new(r"(?s)```bash:verify\s*\n(.*?)\n```")?;

    let mut errors = Vec::new();
    let mut total_notes = 0usize;

    {
        let mut stmt = con.prepare("SELECT rel_path, type, metadata_json FROM entities")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;

        for row in rows {
            let (rel, note_type, metadata_json) = row?;
            total_notes += 1;

            let meta: Value = serde_json::from_str(&metadata_json)
                .with_context(|| format!("Corrupt metadata_json for '{}'", rel))?;
            let needed: &[&str] = match note_type.as_str() {
                "project" => &["title", "date", "type", "tags", "summary", "status"],
                "system" | "daily" | "agent" | "config" | "script" | "reference" => {
                    &["title", "date", "type", "tags", "summary"]
                }
                _ => &["title", "date", "type", "summary"],
            };
            let missing: Vec<&str> = needed
                .iter()
                .copied()
                .filter(|field| !field_truthy(meta.get(*field)))
                .collect();
            if !missing.is_empty() {
                errors.push(format!(
                    "'{}' [{}]: Missing required field(s): {}",
                    rel,
                    note_type,
                    missing.join(", ")
                ));
            }

            let note_path = vault.join(&rel);
            if !note_path.exists() {
                continue;
            }
            match fs::read_to_string(&note_path) {
                Ok(text) => match parse_frontmatter(&text) {
                    Ok((_, body)) => {
                        // Boundary Check: Ensure bash:verify does not run repo unit tests
                        for cap in verify_re.captures_iter(&body) {
                            let cmd = &cap[1];
                            if repo_test_re.is_match(cmd)
                                && !cmd.contains("[ -")
                                && !cmd.contains("hostname")
                            {
                                errors.push(format!(
                                    "'{}': Boundary violation: bash:verify block invokes repo unit tests ('{}'). Akatsuki tests strictly machine invariants (ports, containers, daemons).",
                                    rel, cmd.trim()
                                ));
                            }
                        }
                    }
                    Err(e) => errors.push(format!("'{}': {}", rel, e)),
                },
                Err(e) => errors.push(format!("'{}': File read error during lint: {}", rel, e)),
            }
        }
    }

    for (rel, abs) in raw_asset_files(vault)? {
        let ext = abs.extension().and_then(|e| e.to_str()).unwrap_or("");
        match ext {
            "yml" | "yaml" => {
                let text = match read_asset(&rel, &abs) {
                    Ok(text) => text,
                    Err(msg) => {
                        errors.push(msg);
                        continue;
                    }
                };
                if let Err(e) = serde_yaml::from_str::<serde_yaml::Value>(&text) {
                    errors.push(format!("'{}': YAML syntax error: {}", rel, e));
                }
            }
            "json" => {
                let text = match read_asset(&rel, &abs) {
                    Ok(text) => text,
                    Err(msg) => {
                        errors.push(msg);
                        continue;
                    }
                };
                if let Err(e) = serde_json::from_str::<serde_json::Value>(&text) {
                    errors.push(format!("'{}': JSON syntax error: {}", rel, e));
                }
            }
            "sh" => {
                if let Ok(out) = Command::new("bash").arg("-n").arg(&abs).output() {
                    if !out.status.success() {
                        errors.push(format!(
                            "'{}': Bash syntax error: {}",
                            rel,
                            String::from_utf8_lossy(&out.stderr).trim()
                        ));
                    }
                }
            }
            // Python syntax cannot be checked in-process; skip silently.
            "py" => {}
            _ => {}
        }
    }

    Ok(LintReport {
        passed: errors.is_empty(),
        total_notes,
        errors,
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LinkReport {
    pub passed: bool,
    pub broken_wikilinks: Vec<(String, String)>,
    pub broken_markdown_links: Vec<(String, String)>,
    pub orphan_notes: Vec<String>,
}

/// Link audit before it is flattened into `LinkReport`: broken links, orphan notes,
/// and the notes a domain MOC fails to index.
struct LinkAudit {
    broken: Vec<(String, String)>,
    broken_markdown: Vec<(String, String)>,
    orphans: Vec<String>,
    unindexed: Vec<(String, String)>,
}

/// Audits the vault's link graph by walking the markdown files themselves.
///
/// The `relations` projection is deliberately not consulted: it is derived from the
/// same files, so a stale projection would make this report disagree with the vault.
fn audit_links(vault: &Path) -> Result<LinkAudit> {
    let notes = linkable_notes(vault)?;
    let vault_canon = fs::canonicalize(vault).unwrap_or_else(|_| vault.to_path_buf());

    let mut stems: HashMap<String, Vec<String>> = HashMap::new();
    let mut rels: HashMap<String, String> = HashMap::new();
    let mut inbound: HashMap<String, HashSet<String>> = HashMap::new();
    for (rel, _) in &notes {
        stems
            .entry(file_stem_lower(rel))
            .or_default()
            .push(rel.clone());
        rels.insert(
            rel.strip_suffix(".md").unwrap_or(rel).to_lowercase(),
            rel.clone(),
        );
        inbound.insert(rel.clone(), HashSet::new());
    }

    let fence_re = Regex::new(r"(?s)```.*?```")?;
    let inline_re = Regex::new(r"`[^`\n]+`")?;
    let wikilink_re = Regex::new(r"\[\[([^\]\|]+)(?:\|[^\]]+)?\]\]")?;
    let mdlink_re = Regex::new(r"\[(?:[^\]]*)\]\(([^)#\s]+)(?:#[^)]*)?\)")?;

    let mut broken = Vec::new();
    let mut broken_markdown = Vec::new();

    for (src_rel, abs) in &notes {
        let text = match fs::read_to_string(abs) {
            Ok(text) => text,
            Err(e) => {
                broken.push((src_rel.clone(), format!("Unreadable file: {e}")));
                continue;
            }
        };

        // Strip code blocks so documented examples are not audited as real links.
        let without_fences = fence_re.replace_all(&text, "");
        let clean = inline_re.replace_all(&without_fences, "");

        for cap in wikilink_re.captures_iter(&clean) {
            let whole = cap.get(0).context("wikilink match without a full span")?;
            // An escaped `\[[` is documentation, not a link. An even number of backslashes means the backslashes are escaped.
            let backslashes = clean[..whole.start()]
                .bytes()
                .rev()
                .take_while(|b| *b == b'\\')
                .count();
            if backslashes % 2 == 1 {
                continue;
            }
            let target = cap[1].trim();
            let without_anchor = target.split('#').next().unwrap_or("").trim();
            if without_anchor.is_empty() {
                continue;
            }
            let target_clean = without_anchor.strip_suffix(".md").unwrap_or(without_anchor);
            if let Some(target_rel) = rels.get(&target_clean.to_lowercase()) {
                if let Some(sources) = inbound.get_mut(target_rel) {
                    sources.insert(src_rel.clone());
                }
            } else if let Some(target_rels) = stems.get(&file_stem_lower(target_clean)) {
                for target_rel in target_rels {
                    if let Some(sources) = inbound.get_mut(target_rel) {
                        sources.insert(src_rel.clone());
                    }
                }
            } else {
                broken.push((src_rel.clone(), target.to_string()));
            }
        }

        for cap in mdlink_re.captures_iter(&clean) {
            let target = cap[1].trim();
            if ["http://", "https://", "file://", "mailto:", "#"]
                .iter()
                .any(|prefix| target.starts_with(prefix))
            {
                continue;
            }
            let joined = abs.parent().unwrap_or(vault).join(target);
            match joined.canonicalize() {
                Ok(resolved) => {
                    if !resolved.is_file() {
                        continue;
                    }
                    match resolved.strip_prefix(&vault_canon) {
                        Ok(rel) => {
                            let target_rel = rel.to_string_lossy().replace('\\', "/");
                            if let Some(sources) = inbound.get_mut(&target_rel) {
                                sources.insert(src_rel.clone());
                            }
                        }
                        Err(_) => continue,
                    }
                }
                Err(_) => broken_markdown.push((src_rel.clone(), target.to_string())),
            }
        }
    }

    let is_root_anchor = |rel: &str| {
        ROOT_ANCHORS
            .iter()
            .any(|anchor| anchor.eq_ignore_ascii_case(rel))
    };

    let mut orphans = Vec::new();
    for (rel, _) in &notes {
        if is_root_anchor(rel) || rel.starts_with("01-Daily/") {
            continue;
        }
        if rel.starts_with("50-Configs/") && !rel.ends_with("Configs-MOC.md") {
            continue;
        }
        if rel.starts_with("60-Scripts/") && !rel.ends_with("Scripts-MOC.md") {
            continue;
        }
        if inbound[rel].is_empty() {
            orphans.push(rel.clone());
        }
    }

    // Graph closure: every note in a primary domain must be reachable from its parent
    // MOC or from INDEX.md.
    let mut unindexed = Vec::new();
    for (rel, _) in &notes {
        if is_root_anchor(rel)
            || rel.starts_with("01-Daily/")
            || DOMAIN_MOCS.iter().any(|(_, moc)| moc == rel)
            || rel == "40-Systems/ADRs/ADRs-MOC.md"
        {
            continue;
        }
        let domain = match rel.split_once('/') {
            Some((domain, _)) => domain,
            None => "",
        };
        let parent_moc = match DOMAIN_MOCS.iter().find(|(known, _)| *known == domain) {
            Some((_, moc)) => *moc,
            None => continue,
        };
        let sources = &inbound[rel];
        let mut indexed = sources.iter().any(|s| s.eq_ignore_ascii_case("index.md"))
            || sources.contains(parent_moc);
        if rel.starts_with("40-Systems/ADRs/") {
            indexed = indexed || sources.contains("40-Systems/ADRs/ADRs-MOC.md");
        }
        if !indexed {
            unindexed.push((rel.clone(), parent_moc.to_string()));
        }
    }

    Ok(LinkAudit {
        broken,
        broken_markdown,
        orphans,
        unindexed,
    })
}

pub fn verify_links(vault: &Path) -> Result<LinkReport> {
    let audit = audit_links(vault)?;
    let passed = audit.broken.is_empty()
        && audit.broken_markdown.is_empty()
        && audit.orphans.is_empty()
        && audit.unindexed.is_empty();

    let mut orphan_notes = audit.orphans;
    orphan_notes.extend(
        audit
            .unindexed
            .iter()
            .map(|(rel, moc)| format!("{rel}: not linked from {moc} or INDEX.md")),
    );

    Ok(LinkReport {
        passed,
        broken_wikilinks: audit.broken,
        broken_markdown_links: audit.broken_markdown,
        orphan_notes,
    })
}

#[derive(Debug, Serialize)]
pub struct ReconcileReport {
    pub dry_run: bool,
    pub actions: Vec<String>,
    pub sync: SyncReport,
}

/// Quotes frontmatter scalars containing `": "`, the shape that makes strict YAML
/// frontmatter unparseable. Returns the note's new content when at least one line
/// needs quoting, leaving every other byte of the file untouched.
fn quote_colon_scalars(text: &str, rel: &str, actions: &mut Vec<String>) -> Option<String> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    if lines.first().is_none_or(|line| !line.starts_with("---")) {
        return None;
    }
    let closing = (1..lines.len()).find(|&i| lines[i].trim() == "---")?;
    let fm_start = lines[0].len();
    let fm_end: usize = lines[..closing].iter().map(|line| line.len()).sum();

    let mut changed = false;
    let mut rewritten = String::with_capacity(fm_end - fm_start);
    for line in text[fm_start..fm_end].split_inclusive('\n') {
        // Keep the line's own terminator: only the quoted lines may change.
        let body = line.strip_suffix('\n').unwrap_or(line);
        let body = body.strip_suffix('\r').unwrap_or(body);
        let terminator = &line[body.len()..];

        let stripped = body.trim();
        if stripped.contains(':') && !stripped.starts_with('-') && !stripped.starts_with('#') {
            if let Some((key, raw_value)) = stripped.split_once(':') {
                let value = raw_value.trim();
                let already_quoted = (value.starts_with('"') && value.ends_with('"'))
                    || (value.starts_with('\'') && value.ends_with('\''));
                let is_flow_mapping = value.starts_with('{') && value.ends_with('}');
                let is_flow_sequence = value.starts_with('[') && value.ends_with(']');
                if value.contains(": ") && !already_quoted && !is_flow_mapping && !is_flow_sequence
                {
                    let indent = &body[..body.len() - body.trim_start().len()];
                    rewritten.push_str(&format!(
                        "{indent}{key}: \"{}\"{terminator}",
                        value.replace('"', "\\\"")
                    ));
                    actions.push(format!("Auto-quoted frontmatter field '{key}' in {rel}"));
                    changed = true;
                    continue;
                }
            }
        }
        rewritten.push_str(line);
    }

    if !changed {
        return None;
    }
    let mut new_content = String::with_capacity(text.len() + rewritten.len());
    new_content.push_str(&text[..fm_start]);
    new_content.push_str(&rewritten);
    new_content.push_str(&text[fm_end..]);
    Some(new_content)
}

/// Reconciles the vault with its projection: quotes colon-bearing frontmatter scalars,
/// appends unindexed notes to their parent MOC, then synchronizes the index.
///
/// Every write is committed under `VaultLock` through a tmp-file rename; `dry_run`
/// performs the full scan, reports the actions it would take, and writes nothing.
pub fn reconcile_vault(vault: &Path, dry_run: bool) -> Result<ReconcileReport> {
    let mut actions = Vec::new();

    for (rel, abs) in linkable_notes(vault)? {
        let text = match fs::read_to_string(&abs) {
            Ok(text) => text,
            // An unreadable note is skipped rather than aborting the heal of every
            // other note in the vault.
            Err(_) => continue,
        };
        let new_content = match quote_colon_scalars(&text, &rel, &mut actions) {
            Some(content) => content,
            None => continue,
        };
        if dry_run {
            continue;
        }
        {
            let _lock = VaultLock::acquire(vault)?;
            write_atomic(&abs, &new_content)?;
        }
    }

    for (rel, parent_moc_rel) in audit_links(vault)?.unindexed {
        let parent_moc = vault.join(&parent_moc_rel);
        if !parent_moc.exists() {
            continue;
        }
        let mut moc_text = fs::read_to_string(&parent_moc)
            .with_context(|| format!("Failed to read MOC {}", parent_moc.display()))?;
        let stem = file_stem_of(&rel);
        let target_link = format!("[[{}]]", rel.strip_suffix(".md").unwrap_or(&rel));
        let stem_link = format!("[[{stem}]]");
        if moc_text.contains(&target_link) || moc_text.contains(&stem_link) {
            continue;
        }

        actions.push(format!(
            "Appended unindexed note '{rel}' to {parent_moc_rel}"
        ));
        if dry_run {
            continue;
        }
        if !moc_text.ends_with('\n') {
            moc_text.push('\n');
        }
        moc_text.push_str(&format!("- {target_link}\n"));
        {
            let _lock = VaultLock::acquire(vault)?;
            write_atomic(&parent_moc, &moc_text)?;
        }
    }

    let mut con = open_cache_db(vault)?;
    let sync = if dry_run {
        sync_vault_index_dry_run(vault, &mut con)?
    } else {
        sync_vault_index(vault, &mut con)?
    };

    Ok(ReconcileReport {
        dry_run,
        actions,
        sync,
    })
}

/// File stem as written on disk — MOC entries link by the note's own capitalisation.
fn file_stem_of(rel: &str) -> String {
    Path::new(rel)
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_else(|| rel.to_string())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestItem {
    pub source: String,
    pub command: String,
    pub passed: bool,
    pub executed: bool,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestReport {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub results: Vec<TestItem>,
}

pub fn run_verification_tests(
    vault: &Path,
    note_filter: Option<&str>,
    dry_run: bool,
) -> Result<TestReport> {
    let mut con = open_cache_db(vault)?;
    let _ = sync_vault_index(vault, &mut con)?;

    let sql = if note_filter.is_some() {
        "SELECT source_rel, command FROM verifications WHERE source_rel LIKE ?1"
    } else {
        "SELECT source_rel, command FROM verifications"
    };

    let mut stmt = con.prepare(sql)?;
    let rows: Vec<(String, String)> = if let Some(filt) = note_filter {
        let pat = format!("%{}%", filt);
        stmt.query_map([pat], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };

    if note_filter.is_some() && rows.is_empty() {
        bail!(
            "No 'bash:verify' invariants found for note filter '{}'",
            note_filter.unwrap_or_default()
        );
    }

    let mut results = Vec::new();
    let mut passed = 0;
    let mut failed = 0;
    for (src, cmd) in rows {
        if dry_run {
            results.push(TestItem {
                source: src,
                command: cmd,
                passed: false,
                executed: false,
                exit_code: 0,
                stdout: "[DRY-RUN - not executed]".to_string(),
                stderr: String::new(),
            });
            continue;
        }

        let (code, stdout, stderr) = run_invariant(&cmd, vault);
        let ok = code == 0;
        if ok {
            passed += 1;
        } else {
            failed += 1;
        }
        results.push(TestItem {
            source: src,
            command: cmd,
            passed: ok,
            executed: true,
            exit_code: code,
            stdout,
            stderr,
        });
    }

    Ok(TestReport {
        total: results.len(),
        passed,
        failed,
        results,
    })
}

/// Security gate for invariant assertion commands.
/// Rejects dangerous, destructive, or system-modifying operations before bash execution.
fn is_dangerous_invariant_command(command: &str) -> bool {
    // 1. Output redirection (> or >>), excluding standard discards to /dev/null and fd duplicates
    let sanitized_redir = command
        .replace(">/dev/null", "")
        .replace("> /dev/null", "")
        .replace("2>&1", "")
        .replace(">&2", "")
        .replace("1>&2", "");
    if sanitized_redir.contains('>') {
        return true;
    }

    // 2. Fork bombs
    if command.contains(":(){ :|:& };:") || command.contains(":(){:|:&};:") {
        return true;
    }

    let lower = command.to_lowercase();

    // 3. Low-level disk or filesystem manipulation
    if lower.contains("dd if=") || lower.contains("mkfs") {
        return true;
    }

    // 4. Token-based matching for destructive commands
    let is_boundary = |c: char| {
        c.is_whitespace() || matches!(c, ';' | '|' | '&' | '(' | ')' | '`' | '$' | '<')
    };
    for token in lower.split(is_boundary) {
        let trimmed = token.trim_matches(|c: char| c == '"' || c == '\'' || c == '\\');
        match trimmed {
            "rm" | "rmdir" | "reboot" | "shutdown" | "passwd" | "poweroff" | "halt" | "init" => {
                if trimmed == "init" {
                    if lower.contains("init 0") || lower.contains("init 6") {
                        return true;
                    }
                } else {
                    return true;
                }
            }
            _ => {}
        }
    }

    // 5. Additional explicit patterns
    if lower.contains("rm -rf") || lower.contains("rm -r") || lower.contains("rm -f") {
        return true;
    }

    false
}

/// Executes one invariant under a hard ceiling. A wedged assertion (`nc -z`, an
/// unbounded `curl`) must never stall the caller, let alone the MCP request loop.
fn run_invariant(command: &str, cwd: &Path) -> (i32, String, String) {
    if is_dangerous_invariant_command(command) {
        return (
            1,
            String::new(),
            "Security violation: command contains prohibited destructive or system-modifying operations".to_string(),
        );
    }

    use std::io::Read;
    use std::os::unix::process::CommandExt;

    let script = format!("set -eo pipefail;\n{}", command);
    let spawn = Command::new("bash")
        .arg("-c")
        .arg(&script)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn();

    let mut child = match spawn {
        Ok(child) => child,
        Err(e) => return (1, String::new(), format!("Failed to spawn bash: {}", e)),
    };

    let (tx_out, rx_out) = std::sync::mpsc::channel();
    let (tx_err, rx_err) = std::sync::mpsc::channel();
    let _stdout_handle = child.stdout.take().map(|pipe| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.take(2 * 1024 * 1024).read_to_end(&mut buf);
            let _ = tx_out.send(buf);
        })
    });
    let _stderr_handle = child.stderr.take().map(|pipe| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.take(2 * 1024 * 1024).read_to_end(&mut buf);
            let _ = tx_err.send(buf);
        })
    });

    let kill_group = |child: &mut std::process::Child| {
        let _ = child.kill();
        let _ = Command::new("kill")
            .arg("-KILL")
            .arg(format!("-{}", child.id()))
            .status();
        let _ = child.wait();
    };

    let timeout = invariant_timeout();
    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(e) => {
                eprintln!("Failed to await invariant: {}", e);
                kill_group(&mut child);
                return (
                    1,
                    String::new(),
                    format!("Failed to await invariant: {}", e),
                );
            }
        }
        if Instant::now() >= deadline {
            timed_out = true;
            kill_group(&mut child);
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let stdout = rx_out
        .recv_timeout(Duration::from_millis(500))
        .unwrap_or_default();
    let stderr = rx_err
        .recv_timeout(Duration::from_millis(500))
        .unwrap_or_default();

    if timed_out {
        return (
            124,
            String::from_utf8_lossy(&stdout).trim().to_string(),
            format!(
                "Timed out after {}s; process group killed",
                timeout.as_secs()
            ),
        );
    }

    let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(1);
    (
        code,
        String::from_utf8_lossy(&stdout).trim().to_string(),
        String::from_utf8_lossy(&stderr).trim().to_string(),
    )
}
