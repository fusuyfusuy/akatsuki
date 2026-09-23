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

/// S-03: Public mutation entrypoints must reject writing to internal `.akatsuki/` metadata files or `.akatsuki.lock`.
#[test]
fn mutations_reject_internal_akatsuki_paths() {
    let dir_handle = tempdir().unwrap();
    let vault = dir_handle.path();

    // 1. write_note to .akatsuki/foo, .akatsuki.lock, or ./.akatsuki/cache.db must fail
    let err1 = akatsuki::mutations::write_note(vault, ".akatsuki/foo", "content", true, false);
    assert!(err1.is_err(), "write_note to .akatsuki/foo should fail");
    let msg1 = format!("{}", err1.unwrap_err());
    assert!(
        msg1.contains("Cannot mutate internal akatsuki metadata files"),
        "Unexpected error message: {}",
        msg1
    );

    let err2 = akatsuki::mutations::write_note(vault, ".akatsuki.lock", "content", true, false);
    assert!(err2.is_err(), "write_note to .akatsuki.lock should fail");
    let msg2 = format!("{}", err2.unwrap_err());
    assert!(
        msg2.contains("Cannot mutate internal akatsuki metadata files"),
        "Unexpected error message: {}",
        msg2
    );

    let err3 =
        akatsuki::mutations::write_note(vault, "./.akatsuki/cache.db", "content", true, false);
    assert!(
        err3.is_err(),
        "write_note to ./.akatsuki/cache.db should fail"
    );
    let msg3 = format!("{}", err3.unwrap_err());
    assert!(
        msg3.contains("Cannot mutate internal akatsuki metadata files"),
        "Unexpected error message: {}",
        msg3
    );

    // 2. set_note_property targeting .akatsuki/foo or .akatsuki.lock must fail
    let err4 = akatsuki::mutations::set_note_property(vault, ".akatsuki/foo", "title", "Bad");
    assert!(
        err4.is_err(),
        "set_note_property to .akatsuki/foo should fail"
    );
    let msg4 = format!("{}", err4.unwrap_err());
    assert!(
        msg4.contains("Cannot mutate internal akatsuki metadata files"),
        "Unexpected error message: {}",
        msg4
    );

    let err5 = akatsuki::mutations::set_note_property(vault, ".akatsuki.lock", "title", "Bad");
    assert!(
        err5.is_err(),
        "set_note_property to .akatsuki.lock should fail"
    );
    let msg5 = format!("{}", err5.unwrap_err());
    assert!(
        msg5.contains("Cannot mutate internal akatsuki metadata files"),
        "Unexpected error message: {}",
        msg5
    );
}

/// S-04: `write_atomic` must preserve file permissions (e.g. 0755 or 0600) on unix systems when replacing an existing file.
#[test]
fn write_atomic_preserves_file_permissions() {
    let dir_handle = tempdir().unwrap();
    let file_path = dir_handle.path().join("script.sh");

    // Write initial file and set executable mode 0755
    fs::write(&file_path, "#!/bin/sh\necho initial\n").unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o755);
        fs::set_permissions(&file_path, perms).unwrap();

        let before_mode = fs::metadata(&file_path).unwrap().permissions().mode();
        assert_eq!(before_mode & 0o777, 0o755);

        // Replace file with write_atomic
        akatsuki::storage::write_atomic(&file_path, "#!/bin/sh\necho updated\n").unwrap();

        let after_mode = fs::metadata(&file_path).unwrap().permissions().mode();
        assert_eq!(
            after_mode & 0o777,
            0o755,
            "write_atomic must preserve 0755 executable permissions"
        );
    }

    #[cfg(not(unix))]
    {
        akatsuki::storage::write_atomic(&file_path, "#!/bin/sh\necho updated\n").unwrap();
        assert_eq!(
            fs::read_to_string(&file_path).unwrap(),
            "#!/bin/sh\necho updated\n"
        );
    }
}

/// S-05: Lines starting with `#` inside markdown code blocks used to be treated as ATX
/// headings in vector chunking, creating bogus breadcrumbs and fragmenting chunks.
#[test]
fn vector_chunking_ignores_code_block_comments() {
    let content = r#"---
title: Sample Code Note
summary: Testing code fence chunking
tags: [test, code]
---
# Primary Section

Introductory explanation.

```python
# comment that looks like a heading
def foo():
    # another comment
    return True
```

```bash
# bash comment
echo "hello"
```

~~~
# tilde block comment
echo "tilde"
~~~

## Secondary Section

After code blocks.
"#;

    let chunks = akatsuki::vectors::chunk_note("40-Systems/Code.md", content)
        .expect("chunk_note should succeed");

    let breadcrumbs: Vec<&str> = chunks.iter().map(|c| c.breadcrumb.as_str()).collect();

    assert!(
        breadcrumbs.contains(&"Primary Section"),
        "Primary Section should be a breadcrumb"
    );
    assert!(
        breadcrumbs.contains(&"Secondary Section"),
        "Secondary Section should be a breadcrumb"
    );
    assert!(
        !breadcrumbs.contains(&"comment that looks like a heading"),
        "python comment must not be treated as a heading breadcrumb"
    );
    assert!(
        !breadcrumbs.contains(&"another comment"),
        "nested comment must not be treated as a heading breadcrumb"
    );
    assert!(
        !breadcrumbs.contains(&"bash comment"),
        "bash comment must not be treated as a heading breadcrumb"
    );
    assert!(
        !breadcrumbs.contains(&"tilde block comment"),
        "tilde fence comment must not be treated as a heading breadcrumb"
    );

    let primary_chunk = chunks
        .iter()
        .find(|c| c.breadcrumb == "Primary Section")
        .expect("Primary Section chunk exists");
    assert!(
        primary_chunk
            .embed_text
            .contains("# comment that looks like a heading"),
        "code block comment should be retained inside section embed_text"
    );
    assert!(
        primary_chunk.embed_text.contains("def foo():"),
        "code block body should be retained inside section embed_text"
    );
}

/// S-06: Read errors during file scanning (e.g. invalid UTF-8 or I/O failure) used to
/// silently drop the file from scanned_files and trigger deletion from cache.db.
#[test]
fn reconcile_read_errors_do_not_delete_cache_records() {
    let vault = vault_with(
        "40-Systems",
        "Preserve.md",
        "---\ntitle: Preserve\ndate: 2026-09-21\ntype: system\nsummary: Important system\nstatus: live\n---\n# Preserve\n\nPreserved content.\n",
    );

    // 1. Initial reconcile indexes the file into SQLite cache
    run(vault.path(), &["reconcile"])
        .success()
        .stdout(predicate::str::contains("Vault reconciliation completed"));

    // Verify entity exists in cache
    let db_path = vault.path().join(".akatsuki/cache.db");
    let con = rusqlite::Connection::open(&db_path).unwrap();
    let count: i64 = con
        .query_row(
            "SELECT count(*) FROM entities WHERE stem = 'Preserve'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 1,
        "Preserve entity must exist after initial reconcile"
    );

    // 2. Overwrite file with invalid UTF-8 bytes to simulate read failure
    let file_path = vault.path().join("40-Systems/Preserve.md");
    fs::write(&file_path, [0xFF, 0xFE, 0xFD]).unwrap();

    // 3. Reconcile again: should report parse/read error, NOT delete the record
    let res = run(vault.path(), &["reconcile"]).success();
    let stdout = String::from_utf8_lossy(&res.get_output().stdout);
    assert!(
        stdout.contains("Preserve.md")
            || stdout.contains("read error")
            || stdout.contains("stream did not contain valid UTF-8"),
        "reconcile output should mention the errored note: {}",
        stdout
    );

    // 4. Assert that Preserve still exists in cache.db
    let count_after: i64 = con
        .query_row(
            "SELECT count(*) FROM entities WHERE stem = 'Preserve'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        count_after, 1,
        "Preserve entity must NOT be deleted from cache.db on read error"
    );

    let meta_count: i64 = con
        .query_row(
            "SELECT count(*) FROM file_meta WHERE rel_path = '40-Systems/Preserve.md'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        meta_count, 1,
        "file_meta must NOT be deleted from cache.db on read error"
    );
}

/// S-02: Notes missing from note_vectors (e.g. when models were installed post-hoc)
/// must be detected and queued for embedding during reconcile even if files are unchanged.
#[test]
fn reconcile_detects_notes_missing_vectors() {
    let vault = vault_with(
        "40-Systems",
        "Delayed.md",
        "---\ntitle: Delayed\ndate: 2026-09-21\ntype: system\nsummary: Delayed model setup\nstatus: live\n---\n# Delayed\n\nNote content to embed.\n",
    );

    // Initial reconcile
    run(vault.path(), &["reconcile"])
        .success()
        .stdout(predicate::str::contains("Vault reconciliation completed"));

    let db_path = vault.path().join(".akatsuki/cache.db");
    let con = rusqlite::Connection::open(&db_path).unwrap();

    // Clear note_vectors to simulate notes indexed prior to model setup
    con.execute("DELETE FROM note_vectors", []).unwrap();
    let vec_count: i64 = con
        .query_row("SELECT count(*) FROM note_vectors", [], |r| r.get(0))
        .unwrap();
    assert_eq!(vec_count, 0);

    // Second reconcile with unchanged files:
    // With S-02 fix, missing vector files are detected and added to vector_sources.
    let res = run(vault.path(), &["reconcile"]).success();
    let stdout = String::from_utf8_lossy(&res.get_output().stdout);

    if akatsuki::vectors::feature_enabled() {
        assert!(
            stdout.contains("vectors: embedded")
                || stdout.contains("vectors: skipped 1 changed note"),
            "reconcile must attempt vector sync for notes missing from note_vectors: {}",
            stdout
        );
    }
}

/// `is_raw_path` recognizes raw files in subdirectories, and `write_note` does not
/// force-append `.md` to them.
#[test]
fn is_raw_path_recognizes_subdirectories_and_prevents_forced_md() {
    assert!(akatsuki::storage::is_raw_path("Dockerfile"));
    assert!(akatsuki::storage::is_raw_path("apps/Dockerfile"));
    assert!(akatsuki::storage::is_raw_path("20-Projects/web/Dockerfile"));
    assert!(akatsuki::storage::is_raw_path("services/caddy/Caddyfile"));
    assert!(akatsuki::storage::is_raw_path("sub/dir/Makefile"));
    assert!(akatsuki::storage::is_raw_path("config/.env.example"));
    assert!(!akatsuki::storage::is_raw_path("20-Projects/app/notes.md"));
    assert!(!akatsuki::storage::is_raw_path("Dockerfile.txt"));

    let dir_handle = tempdir().unwrap();
    let vault = dir_handle.path();

    // write_note for apps/Dockerfile should write apps/Dockerfile directly without force-appending .md
    let res = akatsuki::mutations::write_note(
        vault,
        "apps/Dockerfile",
        "FROM rust:alpine\n",
        false,
        false,
    );
    assert!(
        res.is_ok(),
        "write_note must succeed for raw path: {:?}",
        res
    );
    assert!(
        vault.join("apps/Dockerfile").exists(),
        "apps/Dockerfile must exist"
    );
    assert!(
        !vault.join("apps/Dockerfile.md").exists(),
        "apps/Dockerfile.md must NOT exist"
    );

    // Also verify via MCP akatsuki_write_note
    let req = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"akatsuki_write_note","arguments":{"path":"20-Projects/app/Caddyfile","content":"localhost { respond OK }"}}}"#;
    let mut cmd = Command::cargo_bin("akatsuki").unwrap();
    cmd.arg("--vault")
        .arg(vault)
        .arg("mcp")
        .write_stdin(format!("{}\n", req))
        .assert()
        .success()
        .stdout(predicate::str::contains("\"isError\":false"));

    assert!(
        vault.join("20-Projects/app/Caddyfile").exists(),
        "20-Projects/app/Caddyfile must exist"
    );
    assert!(
        !vault.join("20-Projects/app/Caddyfile.md").exists(),
        "20-Projects/app/Caddyfile.md must NOT exist"
    );
}

/// MCP tool definitions in `get_tool_definitions()` expose parameter aliases.
#[test]
fn mcp_tool_definitions_expose_parameter_aliases() {
    let tools = akatsuki::mcp::get_tool_definitions();

    let find_tool = |name: &str| {
        tools
            .iter()
            .find(|t| t.get("name").and_then(|v| v.as_str()) == Some(name))
            .unwrap_or_else(|| panic!("Tool '{}' not found in get_tool_definitions", name))
    };

    // akatsuki_read must expose 'note', 'path', and 'target'
    let read_tool = find_tool("akatsuki_read");
    let read_props = read_tool["inputSchema"]["properties"].as_object().unwrap();
    assert!(
        read_props.contains_key("note"),
        "akatsuki_read must have 'note'"
    );
    assert!(
        read_props.contains_key("path"),
        "akatsuki_read must have 'path'"
    );
    assert!(
        read_props.contains_key("target"),
        "akatsuki_read must have 'target'"
    );

    // akatsuki_test must expose 'note' and 'target'
    let test_tool = find_tool("akatsuki_test");
    let test_props = test_tool["inputSchema"]["properties"].as_object().unwrap();
    assert!(
        test_props.contains_key("note"),
        "akatsuki_test must have 'note'"
    );
    assert!(
        test_props.contains_key("target"),
        "akatsuki_test must have 'target'"
    );

    // akatsuki_map must expose 'target', 'note', and 'path'
    let map_tool = find_tool("akatsuki_map");
    let map_props = map_tool["inputSchema"]["properties"].as_object().unwrap();
    assert!(
        map_props.contains_key("target"),
        "akatsuki_map must have 'target'"
    );
    assert!(
        map_props.contains_key("note"),
        "akatsuki_map must have 'note'"
    );
    assert!(
        map_props.contains_key("path"),
        "akatsuki_map must have 'path'"
    );

    // akatsuki_contract must expose 'note', 'target', and 'path'
    let contract_tool = find_tool("akatsuki_contract");
    let contract_props = contract_tool["inputSchema"]["properties"]
        .as_object()
        .unwrap();
    assert!(
        contract_props.contains_key("note"),
        "akatsuki_contract must have 'note'"
    );
    assert!(
        contract_props.contains_key("target"),
        "akatsuki_contract must have 'target'"
    );
    assert!(
        contract_props.contains_key("path"),
        "akatsuki_contract must have 'path'"
    );

    // akatsuki_blast must expose 'target', 'note', and 'path'
    let blast_tool = find_tool("akatsuki_blast");
    let blast_props = blast_tool["inputSchema"]["properties"].as_object().unwrap();
    assert!(
        blast_props.contains_key("target"),
        "akatsuki_blast must have 'target'"
    );
    assert!(
        blast_props.contains_key("note"),
        "akatsuki_blast must have 'note'"
    );
    assert!(
        blast_props.contains_key("path"),
        "akatsuki_blast must have 'path'"
    );

    // Also verify via MCP JSON-RPC tools/list
    let vault = tempdir().unwrap();
    let req = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;
    let mut cmd = Command::cargo_bin("akatsuki").unwrap();
    cmd.arg("--vault")
        .arg(vault.path())
        .arg("mcp")
        .write_stdin(format!("{}\n", req))
        .assert()
        .success()
        .stdout(predicate::str::contains("\"target\""))
        .stdout(predicate::str::contains("\"path\""));
}

/// S-07: Invariant execution caps captured stdout buffer to 2 MB to prevent memory exhaustion.
#[test]
fn run_invariant_caps_output_buffer_at_2mb() {
    let vault = vault_with(
        "40-Systems",
        "CapBuffer.md",
        "---\ntitle: CapBuffer\ndate: 2026-09-21\ntype: system\nsummary: s\nstatus: live\n---\n# CapBuffer\n\n```bash:verify\nhead -c 3145728 /dev/zero | tr '\\0' 'A'\n```\n",
    );
    run(vault.path(), &["reconcile"]).success();

    let started = Instant::now();
    let rep = akatsuki::verify::run_verification_tests(vault.path(), Some("CapBuffer"), false)
        .expect("run_verification_tests failed");

    assert_eq!(rep.results.len(), 1);
    let item = &rep.results[0];
    assert!(
        item.stdout.len() <= 2 * 1024 * 1024,
        "stdout buffer length {} exceeded 2MB limit",
        item.stdout.len()
    );
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "invariant execution took too long: {:?}",
        started.elapsed()
    );
}

/// Frontmatter auto-quoting must not quote flow mappings or flow sequences containing colons.
#[test]
fn quote_colon_scalars_preserves_flow_mappings_and_arrays() {
    let dir_handle = tempdir().unwrap();
    let projects = dir_handle.path().join("20-Projects");
    fs::create_dir_all(&projects).unwrap();

    let note_content = "---\ntitle: Flow Test\ndate: 2026-09-23\ntype: project\nsummary: Test note\nstatus: live\nflow_map: { key: value, port: 8080 }\nports: [\"80: 8080\", \"443: 8443\"]\n---\n# Flow Test\n";
    let note_path = projects.join("flow-test.md");
    fs::write(&note_path, note_content).unwrap();

    run(dir_handle.path(), &["reconcile"]).success();

    let after = fs::read_to_string(&note_path).unwrap();
    assert!(
        after.contains("flow_map: { key: value, port: 8080 }"),
        "flow mapping must remain unquoted, but got:\n{}",
        after
    );
    assert!(
        after.contains("ports: [\"80: 8080\", \"443: 8443\"]"),
        "flow array must remain unquoted, but got:\n{}",
        after
    );
}

/// `parse_wikilinks` must ignore wikilinks inside markdown code fences.
#[test]
fn parse_wikilinks_ignores_code_fences() {
    let body = "# Example\n\n```markdown\nHere is an example link: [[PhantomDep]]\n```\n\nHere is a real link: [[RealDep]].";
    let links = akatsuki::index::parse_wikilinks(body);
    assert_eq!(links, vec!["RealDep".to_string()]);
}
