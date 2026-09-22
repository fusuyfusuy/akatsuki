//! Regression suite for the Rust rewrite's crash, corruption and containment defects.
//!
//! Every test here reproduces a defect that shipped in v0.2.0 and must fail again
//! if the fix is reverted.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::time::{Duration, Instant};
use tempfile::tempdir;

fn vault_with(dir: &str, name: &str, content: &str) -> tempfile::TempDir {
    let dir_handle = tempdir().unwrap();
    let domain = dir_handle.path().join(dir);
    fs::create_dir_all(&domain).unwrap();
    fs::write(domain.join(name), content).unwrap();
    dir_handle
}

fn run(vault: &std::path::Path, args: &[&str]) -> assert_cmd::assert::Assert {
    let mut cmd = Command::cargo_bin("akatsuki").unwrap();
    cmd.arg("--vault").arg(vault);
    for a in args {
        cmd.arg(a);
    }
    cmd.assert()
}

/// A compact hit whose snippet exceeds the truncation width in bytes used to panic
/// with "byte index 160 is not a char boundary" and kill the MCP server.
#[test]
fn compact_search_truncates_multibyte_snippets() {
    let long_token = "ü".repeat(30);
    let body = format!(
        "# p\n{} {} needlepanicmarker\n",
        "x".repeat(24),
        [long_token.as_str(); 6].join(" ")
    );
    let vault = vault_with(
        "40-Systems",
        "Multibyte.md",
        &format!(
            "---\ntitle: Multibyte\ndate: 2026-09-21\ntype: system\nsummary: s\nstatus: live\n---\n{}",
            body
        ),
    );

    run(vault.path(), &["search", "needlepanicmarker", "--compact"])
        .success()
        .stdout(predicate::str::contains("Snippet"));
}

/// `set` on a note whose YAML does not parse used to rewrite the file into a
/// doubled frontmatter block (old metadata demoted to body text).
#[test]
fn set_property_refuses_unparseable_frontmatter() {
    let original = "---\ntitle: Bad: Note\ndate: 2026-09-21\ntype: system\nsummary: s\nstatus: live\n---\n# Bad\n\nbody line\n";
    let vault = vault_with("40-Systems", "Bad.md", original);

    run(
        vault.path(),
        &["set", "Bad", "--key", "status", "--value", "retired"],
    )
    .failure()
    .stderr(predicate::str::contains("invalid frontmatter"));

    assert_eq!(
        fs::read_to_string(vault.path().join("40-Systems/Bad.md")).unwrap(),
        original,
        "refused mutation must leave the note byte-identical"
    );
}

/// `daily --date ../../secret` used to read an arbitrary `.md` file outside the vault.
#[test]
fn daily_rejects_path_traversal_dates() {
    let vault = vault_with("01-Daily", "2026-09-21.md", "---\ntype: daily\n---\n");
    let outside = vault.path().parent().unwrap().join("secret-escape.md");
    fs::write(&outside, "OUTSIDE").unwrap();

    run(vault.path(), &["daily", "--date", "../../secret-escape"])
        .failure()
        .stderr(predicate::str::contains("Invalid date"));
}

/// A wedged invariant (`nc -z` against a blackholed port) used to stall the caller
/// indefinitely — and, through MCP, the whole server loop.
#[test]
fn invariant_timeout_reports_failure_instead_of_hanging() {
    let vault = vault_with(
        "40-Systems",
        "Hang.md",
        "---\ntitle: Hang\ndate: 2026-09-21\ntype: system\nsummary: s\nstatus: live\n---\n# Hang\n\n```bash:verify\nsleep 30\n```\n",
    );

    let started = Instant::now();
    let mut cmd = Command::cargo_bin("akatsuki").unwrap();
    cmd.env("AKATSUKI_INVARIANT_TIMEOUT", "1")
        .arg("--vault")
        .arg(vault.path())
        .arg("test")
        .assert()
        .failure()
        .stdout(predicate::str::contains("exit 124"));

    assert!(
        started.elapsed() < Duration::from_secs(20),
        "invariant runner ignored its ceiling: {:?}",
        started.elapsed()
    );
}

/// `akatsuki_test {dry_run:"true"}` used to read the string as absent, flip to
/// false, and execute live assertions against the operator's infrastructure.
#[test]
fn mcp_string_dry_run_flag_is_honoured() {
    let vault = vault_with(
        "40-Systems",
        "Side.md",
        "---\ntitle: Side\ndate: 2026-09-21\ntype: system\nsummary: s\nstatus: live\n---\n# Side\n\n```bash:verify\ntouch /tmp/akatsuki-dry-run-leak\n```\n",
    );
    let _ = fs::remove_file("/tmp/akatsuki-dry-run-leak");

    let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"akatsuki_test","arguments":{"note":"Side","dry_run":"true"}}}"#;

    let mut cmd = Command::cargo_bin("akatsuki").unwrap();
    cmd.arg("--vault")
        .arg(vault.path())
        .arg("mcp")
        .write_stdin(format!("{}\n", request))
        .assert()
        .success()
        .stdout(predicate::str::contains("[DRY-RUN - not executed]"));

    assert!(
        !std::path::Path::new("/tmp/akatsuki-dry-run-leak").exists(),
        "dry run executed the assertion"
    );
}

/// A message without an id is a JSON-RPC notification: replying to it violates the
/// protocol and confuses hosts that match replies by id.
#[test]
fn mcp_never_replies_to_notifications() {
    let vault = vault_with("40-Systems", "Note.md", "---\ntitle: Note\n---\n# Note\n");

    let mut cmd = Command::cargo_bin("akatsuki").unwrap();
    cmd.arg("--vault")
        .arg(vault.path())
        .arg("mcp")
        .write_stdin(
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{}}\n\
             {\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"ping\"}\n",
        )
        .assert()
        .success()
        .stdout(predicate::str::contains("\"id\":7"))
        .stdout(predicate::str::contains("\"id\":null").not());
}

/// A note chunk whose subword token count exceeds BERT's 512 position embeddings
/// used to panic Candle with "index-select invalid index 512 with dim size 512".
#[test]
fn long_note_chunks_do_not_exceed_bert_position_embeddings() {
    let long_body = "word ".repeat(700);
    let vault = vault_with(
        "40-Systems",
        "Long.md",
        &format!(
            "---\ntitle: Long\ndate: 2026-09-21\ntype: system\nsummary: s\nstatus: live\n---\n# Long\n\n{}\n",
            long_body
        ),
    );

    run(vault.path(), &["reconcile"])
        .success()
        .stdout(predicate::str::contains("Vault reconciliation completed"));
}

/// `lint --json` used to exit with code 0 even when schema violations were present.
#[test]
fn lint_json_exits_with_code_1_on_failure() {
    let vault = vault_with(
        "40-Systems",
        "Invalid.md",
        "---\ntitle: Invalid\ntype: system\n---\n# Invalid\n",
    );

    run(vault.path(), &["lint", "--json"])
        .failure()
        .code(1)
        .stdout(predicate::str::contains("\"passed\": false"));
}

/// Lines starting with `#` inside markdown code blocks used to be treated as headings,
/// prematurely truncating sections.
#[test]
fn section_extraction_ignores_comments_inside_code_fences() {
    let content = r#"# Document

## Configuration
Before script
```bash
# comment that looks like a heading
echo "still in configuration"
```
After script

## Next Section
Other content
"#;

    let extracted = akatsuki::storage::extract_section(content, "Configuration")
        .expect("Configuration section should be found");
    assert!(
        extracted.contains("# comment that looks like a heading"),
        "code block comment should remain inside extracted section"
    );
    assert!(
        extracted.contains("After script"),
        "content after code block should remain inside section"
    );
    assert!(
        !extracted.contains("## Next Section"),
        "following section should not be included in extracted section"
    );
}

/// Mutating CTEs (e.g. `WITH del AS (DELETE ... RETURNING *) SELECT * FROM del`)
/// used to bypass the read-only check and mutate SQLite tables. Also verify queries with leading comments succeed.
#[test]
fn execute_sql_query_rejects_mutating_ctes() {
    let vault = vault_with(
        "40-Systems",
        "Note.md",
        "---\ntitle: Note\ndate: 2026-09-21\ntype: system\nsummary: s\nstatus: live\n---\n# Note\n",
    );
    run(vault.path(), &["reconcile"]).success();

    let mutating_sql = "WITH del AS (SELECT 1) DELETE FROM entities";
    let res = akatsuki::search::execute_sql_query(vault.path(), mutating_sql);
    assert!(res.is_err(), "mutating CTE must be rejected");
    let err = res.unwrap_err().to_string();
    assert!(
        err.contains("not read-only") || err.contains("Security violation"),
        "error must indicate security violation or read-only failure: {}",
        err
    );

    // CLI invocation must also fail
    run(vault.path(), &["query", mutating_sql]).failure();

    // Query with leading comment must succeed
    let comment_sql = "-- comment\nSELECT count(*) FROM entities";
    let res_comment = akatsuki::search::execute_sql_query(vault.path(), comment_sql);
    assert!(
        res_comment.is_ok(),
        "query with leading comment should succeed: {:?}",
        res_comment.err()
    );
}

/// Relative vault invocations (e.g. `contained_path(Path::new("."), "../../etc/passwd")`)
/// used to normalize underflow into an uncontained path.
#[test]
fn contained_path_rejects_relative_underflow() {
    let res = akatsuki::storage::contained_path(std::path::Path::new("."), "../../etc/passwd");
    assert_eq!(
        res, None,
        "contained_path on relative vault must reject traversal underflow"
    );
}

/// Notes with malformed YAML frontmatter used to bubble an error up and abort
/// vector chunking across the entire vault.
#[test]
fn chunk_note_succeeds_on_malformed_frontmatter() {
    let content = "---\ntitle: [unclosed sequence\nstatus: live\n---\n# Section Title\nBody content of the note.\n";
    let chunks = akatsuki::vectors::chunk_note("40-Systems/Malformed.md", content);
    assert!(
        chunks.is_ok(),
        "chunk_note should succeed on notes with invalid frontmatter: {:?}",
        chunks.err()
    );
    let chunks = chunks.unwrap();
    assert!(
        !chunks.is_empty(),
        "chunks should be generated for note body despite broken frontmatter"
    );
    assert!(
        chunks[0].embed_text.contains("Body content of the note"),
        "chunk text should contain note body content"
    );
}

/// MCP `akatsuki_set` should accept native JSON numbers, booleans, and objects/arrays
/// rather than failing `as_str()` and setting `""`.
#[test]
fn mcp_set_accepts_native_numbers_and_booleans() {
    let vault = vault_with(
        "40-Systems",
        "App.md",
        "---\ntitle: App\ndate: 2026-09-21\ntype: system\nsummary: s\nstatus: live\n---\n# App\n",
    );

    let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"akatsuki_set","arguments":{"note":"App","key":"port","value":8080}}}"#;

    let mut cmd = Command::cargo_bin("akatsuki").unwrap();
    cmd.arg("--vault")
        .arg(vault.path())
        .arg("mcp")
        .write_stdin(format!("{}\n", request))
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Property 'port' set on note 'App'",
        ));

    let content = fs::read_to_string(vault.path().join("40-Systems/App.md")).unwrap();
    assert!(
        content.contains("port: 8080"),
        "note frontmatter must contain 'port: 8080', got:\n{}",
        content
    );
}

/// Invariant processes producing large output (>128 KB) used to stall on full OS pipe buffers
/// and trigger false SIGKILL timeouts. Background draining threads keep the pipe drained.
#[test]
fn run_invariant_drains_large_output_without_timeout() {
    let vault = vault_with(
        "40-Systems",
        "Large.md",
        "---\ntitle: Large\ndate: 2026-09-21\ntype: system\nsummary: s\nstatus: live\n---\n# Large\n\n```bash:verify\nhead -c 131072 /dev/zero | tr '\\0' 'A'\n```\n",
    );
    run(vault.path(), &["reconcile"]).success();

    let started = Instant::now();
    let mut cmd = Command::cargo_bin("akatsuki").unwrap();
    cmd.env("AKATSUKI_INVARIANT_TIMEOUT", "5")
        .arg("--vault")
        .arg(vault.path())
        .arg("test")
        .assert()
        .success()
        .stdout(predicate::str::contains("1 PASSED, 0 FAILED"));

    assert!(
        started.elapsed() < Duration::from_secs(15),
        "large output invariant took too long: {:?}",
        started.elapsed()
    );
}

/// Numeric array indexing in `get_keypath` allows resolving elements in list fields
/// such as `tags.0` or `tags.1`.
#[test]
fn get_keypath_indexes_arrays() {
    let vault = vault_with(
        "40-Systems",
        "Note.md",
        "---\ntitle: Note\ndate: 2026-09-21\ntype: system\nsummary: s\nstatus: live\ntags:\n  - alpha\n  - beta\n---\n# Note\n",
    );
    run(vault.path(), &["reconcile"]).success();

    let tag0 = akatsuki::search::get_keypath(vault.path(), "Note.tags.0").unwrap();
    assert_eq!(tag0, serde_json::json!("alpha"));

    let tag1 = akatsuki::search::get_keypath(vault.path(), "Note.tags.1").unwrap();
    assert_eq!(tag1, serde_json::json!("beta"));

    // Also via CLI
    run(vault.path(), &["get", "Note.tags.0"])
        .success()
        .stdout(predicate::str::contains("alpha"));
}

/// Wikilinks and declared frontmatter relations pointing to directory paths (e.g. `[[40-Systems/Database]]`)
/// must strip directory prefix, surrounding brackets, and `.md` to resolve against entity stem `Database`.
#[test]
fn wikilink_directory_paths_resolve_to_stem() {
    let dir_handle = tempdir().unwrap();
    let systems = dir_handle.path().join("40-Systems");
    let projects = dir_handle.path().join("20-Projects");
    fs::create_dir_all(&systems).unwrap();
    fs::create_dir_all(&projects).unwrap();

    fs::write(
        systems.join("Database.md"),
        "---\ntitle: Database\ndate: 2026-09-21\ntype: system\nsummary: Primary DB\nstatus: live\n---\n# Database\n",
    ).unwrap();

    fs::write(
        projects.join("App.md"),
        "---\ntitle: App\ndate: 2026-09-21\ntype: project\nsummary: Web App\nstatus: live\nrelations:\n  depends_on:\n    - \"[[40-Systems/Database]]\"\n---\n# App\nConnects to [[40-Systems/Database]].\n",
    ).unwrap();

    run(dir_handle.path(), &["reconcile"]).success();

    let blast = akatsuki::graph::calculate_blast_radius(dir_handle.path(), "Database")
        .expect("blast radius should calculate");
    assert!(
        blast
            .upstream
            .iter()
            .any(|u| u.source_rel == "20-Projects/App.md" && u.relation_type == "depends_on"),
        "declared relation [[40-Systems/Database]] must resolve to stem Database: {:?}",
        blast.upstream
    );
    assert!(
        blast
            .upstream
            .iter()
            .any(|u| u.source_rel == "20-Projects/App.md" && u.relation_type == "references"),
        "body wikilink [[40-Systems/Database]] must resolve to stem Database: {:?}",
        blast.upstream
    );

    let report =
        akatsuki::verify::verify_links(dir_handle.path()).expect("verify_links should succeed");
    assert!(
        report.broken_wikilinks.is_empty(),
        "broken wikilinks should be empty, got: {:?}",
        report.broken_wikilinks
    );
}

/// Duplicate note stems across domain directories (e.g. `20-Projects/api.md` and `40-Systems/api.md`)
/// must be tracked in a multi-map so that neither note is falsely reported as an orphan when linked.
#[test]
fn audit_links_handles_duplicate_stems_across_domains() {
    let dir_handle = tempdir().unwrap();
    let systems = dir_handle.path().join("40-Systems");
    let projects = dir_handle.path().join("20-Projects");
    fs::create_dir_all(&systems).unwrap();
    fs::create_dir_all(&projects).unwrap();

    fs::write(
        systems.join("api.md"),
        "---\ntitle: API System\ndate: 2026-09-21\ntype: system\nsummary: System API\nstatus: live\n---\n# API System\n",
    ).unwrap();

    fs::write(
        projects.join("api.md"),
        "---\ntitle: API Project\ndate: 2026-09-21\ntype: project\nsummary: Project API\nstatus: live\n---\n# API Project\n",
    ).unwrap();

    fs::write(
        dir_handle.path().join("INDEX.md"),
        "---\ntitle: Index\n---\n# Index\n- [[api]]\n",
    )
    .unwrap();

    let report =
        akatsuki::verify::verify_links(dir_handle.path()).expect("verify_links should succeed");
    assert!(
        !report
            .orphan_notes
            .contains(&"20-Projects/api.md".to_string()),
        "20-Projects/api.md must not be marked orphan when [[api]] links to it"
    );
    assert!(
        !report
            .orphan_notes
            .contains(&"40-Systems/api.md".to_string()),
        "40-Systems/api.md must not be marked orphan when [[api]] links to it"
    );
}

/// Two-pass heading lookup in `locate_section` ensures an earlier heading containing the
/// target as a substring (e.g. "Dialog") does not shadow a later exact match (e.g. "log").
#[test]
fn locate_section_prefers_exact_match_over_earlier_substring() {
    let content = "## Dialog\nText 1\n## log\nText 2\n";
    let lines: Vec<&str> = content.lines().collect();
    let (start, _level, end) =
        akatsuki::storage::locate_section(&lines, "log").expect("should locate section 'log'");
    let section_text = lines[start..end].join("\n");
    assert!(
        section_text.contains("Text 2"),
        "locate_section for 'log' must match '## log' (Text 2), not '## Dialog' (Text 1). Got:\n{}",
        section_text
    );
    assert!(
        !section_text.contains("Text 1"),
        "matched section must not contain earlier substring match content"
    );
}

/// `akatsuki map Target --direction invalid` must fail with exit code 2 due to Clap value parser.
#[test]
fn map_invalid_direction_exits_with_code_2() {
    let vault = vault_with(
        "40-Systems",
        "Target.md",
        "---\ntitle: Target\n---\n# Target\n",
    );
    let mut cmd = Command::cargo_bin("akatsuki").unwrap();
    cmd.arg("--vault")
        .arg(vault.path())
        .arg("map")
        .arg("Target")
        .arg("--direction")
        .arg("invalid")
        .assert()
        .failure()
        .code(2);
}

/// MCP parameter aliasing ensures `akatsuki_contract` accepts `target`, and `akatsuki_blast` accepts `note`.
#[test]
fn mcp_parameter_aliasing_contract_blast_map() {
    let dir_handle = tempdir().unwrap();
    let systems = dir_handle.path().join("40-Systems");
    let projects = dir_handle.path().join("20-Projects");
    fs::create_dir_all(&systems).unwrap();
    fs::create_dir_all(&projects).unwrap();

    fs::write(
        systems.join("Database.md"),
        "---\ntitle: Database\ndate: 2026-09-21\ntype: system\nsummary: Primary DB\nstatus: live\nports:\n  - \"5432:5432\"\n---\n# Database\n",
    ).unwrap();

    fs::write(
        projects.join("App.md"),
        "---\ntitle: App\ndate: 2026-09-21\ntype: project\nsummary: Web App\nstatus: live\nports:\n  - \"8080:8080\"\nrelations:\n  depends_on:\n    - \"[[40-Systems/Database]]\"\n---\n# App\nConnects to [[40-Systems/Database]].\n",
    ).unwrap();

    run(dir_handle.path(), &["reconcile"]).success();

    // 1. akatsuki_contract accepts target
    let req_contract = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"akatsuki_contract","arguments":{"target":"App"}}}"#;
    let mut cmd = Command::cargo_bin("akatsuki").unwrap();
    cmd.arg("--vault")
        .arg(dir_handle.path())
        .arg("mcp")
        .write_stdin(format!("{}\n", req_contract))
        .assert()
        .success()
        .stdout(predicate::str::contains("\"isError\":false"))
        .stdout(predicate::str::contains("App"));

    // 2. akatsuki_blast accepts note
    let req_blast = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"akatsuki_blast","arguments":{"note":"Database"}}}"#;
    let mut cmd = Command::cargo_bin("akatsuki").unwrap();
    cmd.arg("--vault")
        .arg(dir_handle.path())
        .arg("mcp")
        .write_stdin(format!("{}\n", req_blast))
        .assert()
        .success()
        .stdout(predicate::str::contains("\"isError\":false"))
        .stdout(predicate::str::contains("upstream"));

    // 3. akatsuki_map accepts note
    let req_map = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"akatsuki_map","arguments":{"note":"App"}}}"#;
    let mut cmd = Command::cargo_bin("akatsuki").unwrap();
    cmd.arg("--vault")
        .arg(dir_handle.path())
        .arg("mcp")
        .write_stdin(format!("{}\n", req_map))
        .assert()
        .success()
        .stdout(predicate::str::contains("\"isError\":false"))
        .stdout(predicate::str::contains("App"));
}

/// `index.md` (lowercase) must satisfy the root MOC reachability check without marking notes unindexed.
#[test]
fn index_md_lowercase_satisfies_moc_reachability() {
    let dir_handle = tempdir().unwrap();
    let projects = dir_handle.path().join("20-Projects");
    fs::create_dir_all(&projects).unwrap();

    fs::write(
        projects.join("my-project.md"),
        "---\ntitle: My Project\ndate: 2026-09-21\ntype: project\nsummary: Test Project\nstatus: live\n---\n# My Project\n",
    ).unwrap();

    // lowercase index.md
    fs::write(
        dir_handle.path().join("index.md"),
        "---\ntitle: Index\n---\n# Index\n- [[my-project]]\n",
    )
    .unwrap();

    let report =
        akatsuki::verify::verify_links(dir_handle.path()).expect("verify_links should succeed");
    assert!(
        report.passed,
        "verify_links should pass for lowercase index.md, but got: {:?}",
        report.orphan_notes
    );
    assert!(
        !report
            .orphan_notes
            .iter()
            .any(|o| o.contains("not linked from")),
        "notes linked from lowercase index.md must not be flagged as unindexed: {:?}",
        report.orphan_notes
    );
}
