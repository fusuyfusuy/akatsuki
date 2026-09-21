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
