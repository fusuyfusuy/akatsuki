use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_cli_version() {
    let mut cmd = Command::cargo_bin("akatsuki").unwrap();
    cmd.arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "akatsuki {}",
            env!("CARGO_PKG_VERSION")
        )));
}

#[test]
fn test_vault_e2e_reconcile_and_read() {
    let dir = tempdir().unwrap();
    let vault_path = dir.path();

    // Create domain directory and note
    let systems_dir = vault_path.join("40-Systems");
    fs::create_dir_all(&systems_dir).unwrap();

    let note_content = r#"---
title: "Test Ingress Service"
date: 2026-09-21
type: system
summary: "High performance ingress router for microservices."
ports:
  - "80:8080"
  - "443:8443"
tags:
  - ingress
  - networking
---

# 🌐 Test Ingress Service

## 📌 Architectural Overview
This service manages ingress routing across all cluster nodes.

## 🩺 Machine Verification
```bash:verify
true
```

## 🔗 Related Notes
- [[INDEX|Master Index]]
"#;

    let note_file = systems_dir.join("Test-Ingress-Service.md");
    fs::write(&note_file, note_content).unwrap();

    // Index via reconcile
    let mut cmd = Command::cargo_bin("akatsuki").unwrap();
    cmd.arg("--vault")
        .arg(vault_path)
        .arg("reconcile")
        .assert()
        .success()
        .stdout(predicate::str::contains("Vault reconciliation completed"));

    // Read with space-separated query
    let mut cmd = Command::cargo_bin("akatsuki").unwrap();
    cmd.arg("--vault")
        .arg(vault_path)
        .arg("read")
        .arg("Test Ingress Service")
        .assert()
        .success()
        .stdout(predicate::str::contains("Test Ingress Service"));

    // Read section with emoji tolerance
    let mut cmd = Command::cargo_bin("akatsuki").unwrap();
    cmd.arg("--vault")
        .arg(vault_path)
        .arg("read")
        .arg("Test Ingress Service")
        .arg("--section")
        .arg("Architectural Overview")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "This service manages ingress routing across all cluster nodes.",
        ));

    // BM25 Search
    let mut cmd = Command::cargo_bin("akatsuki").unwrap();
    cmd.arg("--vault")
        .arg(vault_path)
        .arg("search")
        .arg("microservices")
        .assert()
        .success()
        .stdout(predicate::str::contains("Test Ingress Service"));

    // Verify machine invariants
    let mut cmd = Command::cargo_bin("akatsuki").unwrap();
    cmd.arg("--vault")
        .arg(vault_path)
        .arg("test")
        .assert()
        .success()
        .stdout(predicate::str::contains("1 PASSED"));

    // O(1) Get frontmatter property
    let mut cmd = Command::cargo_bin("akatsuki").unwrap();
    cmd.arg("--vault")
        .arg(vault_path)
        .arg("get")
        .arg("Test-Ingress-Service.summary")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "High performance ingress router for microservices.",
        ));

    // SQL Query
    let mut cmd = Command::cargo_bin("akatsuki").unwrap();
    cmd.arg("--vault")
        .arg(vault_path)
        .arg("query")
        .arg("SELECT title FROM entities WHERE stem = 'Test-Ingress-Service'")
        .assert()
        .success()
        .stdout(predicate::str::contains("Test Ingress Service"));
}
