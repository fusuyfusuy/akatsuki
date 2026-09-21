//! Verification, linting, wikilink audit, and machine invariant execution.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::index::{open_cache_db, sync_vault_index};
use crate::storage::parse_frontmatter;

#[derive(Debug, Serialize, Deserialize)]
pub struct LintReport {
    pub passed: bool,
    pub total_notes: usize,
    pub errors: Vec<String>,
}

pub fn lint_vault(vault: &Path) -> Result<LintReport> {
    let mut con = open_cache_db(vault)?;
    let _ = sync_vault_index(vault, &mut con)?;

    let mut stmt = con.prepare("SELECT rel_path FROM file_meta")?;
    let paths: Vec<String> = stmt.query_map([], |r| r.get(0))?.filter_map(Result::ok).collect();

    let mut errors = Vec::new();
    let total = paths.len();

    let repo_test_re = Regex::new(r"(?i)(pytest|npm test|cargo test|bun test|python3\s+[\w_-]*test[\w_-]*\.py|python3\s+verify_backup\.py)")?;

    for rel in &paths {
        let full_p = vault.join(rel);
        let text = match fs::read_to_string(&full_p) {
            Ok(t) => t,
            Err(e) => {
                errors.push(format!("'{}': Failed to read: {}", rel, e));
                continue;
            }
        };

        if !text.starts_with("---") {
            errors.push(format!("'{}': Missing YAML frontmatter marker '---'", rel));
            continue;
        }

        let parts: Vec<&str> = text.splitn(3, "---").collect();
        if parts.len() < 3 {
            errors.push(format!("'{}': Malformed YAML frontmatter", rel));
            continue;
        }

        let (fm, body) = parse_frontmatter(&text);
        if fm.as_object().is_none() {
            errors.push(format!("'{}': Invalid YAML frontmatter syntax", rel));
            continue;
        }

        let note_type = fm.get("type").and_then(|v| v.as_str()).unwrap_or("note");
        let needed = match note_type {
            "project" => vec!["title", "date", "type", "summary", "status"],
            "system" | "daily" | "agent" | "config" => vec!["title", "date", "type", "summary"],
            _ => vec!["title", "date", "type", "summary"],
        };

        for field in needed {
            if fm.get(field).is_none() {
                errors.push(format!("'{}' [{}]: Missing required metadata field '{}'", rel, note_type, field));
            }
        }

        // Boundary Check: Ensure bash:verify does not run repo unit tests
        let verify_re = Regex::new(r"(?s)```bash:verify\s*\n(.*?)\n```")?;
        for cap in verify_re.captures_iter(&body) {
            let cmd = &cap[1];
            if repo_test_re.is_match(cmd) && !cmd.contains("[ -") && !cmd.contains("hostname") {
                errors.push(format!(
                    "'{}': Boundary violation: bash:verify block invokes repo unit tests ('{}'). Akatsuki tests strictly machine invariants (ports, containers, daemons).",
                    rel, cmd.trim()
                ));
            }
        }
    }

    Ok(LintReport {
        passed: errors.is_empty(),
        total_notes: total,
        errors,
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LinkReport {
    pub passed: bool,
    pub broken_wikilinks: Vec<(String, String)>,
    pub orphan_notes: Vec<String>,
}

pub fn verify_links(vault: &Path) -> Result<LinkReport> {
    let mut con = open_cache_db(vault)?;
    let _ = sync_vault_index(vault, &mut con)?;

    let mut stmt = con.prepare("SELECT rel_path, stem FROM entities")?;
    let mut valid_stems: HashSet<String> = HashSet::new();
    let mut valid_rels: HashSet<String> = HashSet::new();

    let entity_rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for (rel, stem) in entity_rows.flatten() {
        valid_stems.insert(stem.to_lowercase());
        let rel_no_ext = rel.strip_suffix(".md").unwrap_or(&rel).to_lowercase();
        valid_rels.insert(rel_no_ext);
    }

    let mut stmt_rel = con.prepare("SELECT source_rel, target_stem FROM relations WHERE relation_type = 'wikilink'")?;
    let mut broken = Vec::new();
    let mut inbounds: HashMap<String, usize> = HashMap::new();

    let rows = stmt_rel.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for (src, target) in rows.flatten() {
        let t_clean = target.trim().to_lowercase();
        let target_no_md = t_clean.strip_suffix(".md").unwrap_or(&t_clean);
        let target_stem = target_no_md.split('/').last().unwrap_or(target_no_md).to_string();

        if valid_stems.contains(&target_stem) || valid_rels.contains(target_no_md) {
            *inbounds.entry(target_stem).or_insert(0) += 1;
        } else {
            broken.push((src.clone(), target));
        }
    }

    let root_anchors = ["index.md", "operator.md", "agents.md", "readme.md"];
    let mut orphans = Vec::new();

    let mut stmt_all = con.prepare("SELECT rel_path, stem FROM entities")?;
    let all_notes = stmt_all.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;

    for (rel, stem) in all_notes.flatten() {
        let rel_lower = rel.to_lowercase();
        if root_anchors.iter().any(|a| rel_lower.ends_with(a)) || rel_lower.starts_with("01-daily/") {
            continue;
        }
        if !inbounds.contains_key(&stem.to_lowercase()) {
            orphans.push(rel);
        }
    }

    Ok(LinkReport {
        passed: broken.is_empty() && orphans.is_empty(),
        broken_wikilinks: broken,
        orphan_notes: orphans,
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestItem {
    pub source: String,
    pub command: String,
    pub passed: bool,
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
            .filter_map(Result::ok)
            .collect()
    } else {
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .filter_map(Result::ok)
            .collect()
    };

    let mut results = Vec::new();
    let mut passed = 0;
    let mut failed = 0;

    for (src, cmd) in rows {
        if dry_run {
            results.push(TestItem {
                source: src,
                command: cmd,
                passed: true,
                exit_code: 0,
                stdout: "[DRY-RUN - skipped]".to_string(),
                stderr: String::new(),
            });
            passed += 1;
            continue;
        }

        let output = Command::new("bash")
            .arg("-c")
            .arg(&cmd)
            .current_dir(vault)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();

        match output {
            Ok(out) => {
                let code = out.status.code().unwrap_or(1);
                let ok = out.status.success();
                if ok {
                    passed += 1;
                } else {
                    failed += 1;
                }
                results.push(TestItem {
                    source: src,
                    command: cmd,
                    passed: ok,
                    exit_code: code,
                    stdout: String::from_utf8_lossy(&out.stdout).trim().to_string(),
                    stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
                });
            }
            Err(e) => {
                failed += 1;
                results.push(TestItem {
                    source: src,
                    command: cmd,
                    passed: false,
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: format!("Failed to spawn bash: {}", e),
                });
            }
        }
    }

    Ok(TestReport {
        total: results.len(),
        passed,
        failed,
        results,
    })
}
