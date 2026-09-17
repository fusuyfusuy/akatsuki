"""Argument parser and initialization logic for Akatsuki CLI."""

import argparse
import datetime
from pathlib import Path

from akatsuki.constants import DOMAIN_DIRS


def cli_init(args: argparse.Namespace) -> None:
    target = Path(args.path).expanduser().resolve()
    target.mkdir(parents=True, exist_ok=True)

    akatsuki_dir = target / ".akatsuki"
    akatsuki_dir.mkdir(exist_ok=True)

    subdirs = [
        "00-Meta",
        "01-Daily",
        "20-Projects",
        "30-Agents",
        "40-Systems",
        "50-Configs",
        "90-Reference",
        "90-Database",
        "_templates",
    ]
    for s in subdirs:
        (target / s).mkdir(exist_ok=True)

    gitignore = target / ".gitignore"
    if not gitignore.exists():
        gitignore.write_text(
            "# Obsidian workspace & caches\n"
            ".obsidian/workspace*\n"
            ".obsidian/cache/\n"
            ".obsidian/graph.json\n"
            ".obsidian/plugins/*/data.json.bak\n\n"
            "# Temporary & local agent scratch files\n"
            "*.tmp\n"
            ".akatsuki.lock\n"
            ".akatsuki/\n"
            ".drafts/\n"
            ".scratch/\n\n"
            "# Machine-specific AST indexing & runtime sessions\n"
            ".mimori/\n"
            ".pi/\n"
            ".pi-glla/\n"
            ".claude/\n"
            ".codex/\n"
            ".gemini/\n"
            ".omp/\n\n"
            "# Secrets\n"
            ".env\n"
            "*.key\n"
            "*.pem\n"
            "*.secret\n",
            encoding="utf-8",
        )

    today_str = datetime.datetime.now().astimezone().strftime("%Y-%m-%d")

    agents_md = target / "AGENTS.md"
    if not agents_md.exists():
        agents_md.write_text(
            f"---\n"
            f'title: "System Protocol & Living Architecture Invariants"\n'
            f"date: {today_str}\n"
            f"type: agent\n"
            f"tags:\n"
            f"  - protocol\n"
            f"  - architecture\n"
            f"  - invariants\n"
            f'summary: "Master architectural protocol and invariants for autonomous agents."\n'
            f"---\n\n"
            f"# AGENTS.md — System Protocol & Living Architecture Invariants\n\n"
            f"## Core Invariants\n"
            f"- **Architecture at the Boundary**: Keep system contracts explicit and strict.\n"
            f"- **Living Invariant Verification**: All assertions under `bash:verify` must evaluate to exit code 0.\n"
            f"- **Telegraphic Caveman Logging**: Append timestamped ledger entries to daily notes upon completing tasks.\n",
            encoding="utf-8",
        )

    operator_md = target / "OPERATOR.md"
    if not operator_md.exists():
        operator_md.write_text(
            f"---\n"
            f'title: "Operator Profile & Machine Specs"\n'
            f"date: {today_str}\n"
            f"type: system\n"
            f"tags:\n"
            f"  - operator\n"
            f"  - telemetry\n"
            f'summary: "Operator environment, hardware telemetry, and communication contracts."\n'
            f"---\n\n"
            f"# 👤 Operator Profile & Machine Specs\n\n"
            f"## 💻 Hardware & Environment Telemetry\n"
            f"- Host: Local Machine\n"
            f"- Role: Autonomous Agent Execution Substrate\n",
            encoding="utf-8",
        )

    projects_moc = target / "20-Projects" / "Projects-MOC.md"
    if not projects_moc.exists():
        projects_moc.write_text(
            f"---\n"
            f'title: "Projects MOC"\n'
            f"date: {today_str}\n"
            f"type: moc\n"
            f"tags:\n"
            f"  - moc\n"
            f"  - projects\n"
            f'summary: "Master Map of Content for active software projects and repositories."\n'
            f"---\n\n"
            f"# 📁 Projects MOC\n\n"
            f"## 📌 Tracked Projects\n\n"
            f"## 🔗 Related Notes\n"
            f"- [[INDEX]]\n",
            encoding="utf-8",
        )

    systems_moc = target / "40-Systems" / "Systems-MOC.md"
    if not systems_moc.exists():
        systems_moc.write_text(
            f"---\n"
            f'title: "Systems MOC"\n'
            f"date: {today_str}\n"
            f"type: moc\n"
            f"tags:\n"
            f"  - moc\n"
            f"  - systems\n"
            f'summary: "Master Map of Content for infrastructure topology, host systems, and ADRs."\n'
            f"---\n\n"
            f"# 🖥️ Systems MOC\n\n"
            f"## 📌 Systems & Services\n\n"
            f"## 🔗 Related Notes\n"
            f"- [[INDEX]]\n",
            encoding="utf-8",
        )

    daily_moc = target / "01-Daily" / "Daily-MOC.md"
    if not daily_moc.exists():
        daily_moc.write_text(
            f"---\n"
            f'title: "Daily MOC"\n'
            f"date: {today_str}\n"
            f"type: moc\n"
            f"tags:\n"
            f"  - moc\n"
            f"  - daily\n"
            f'summary: "Master Map of Content for operational worklogs and session history."\n'
            f"---\n\n"
            f"# 📅 Daily MOC\n\n"
            f"## 📌 Daily Ledgers\n\n"
            f"## 🔗 Related Notes\n"
            f"- [[INDEX]]\n",
            encoding="utf-8",
        )

    index_md = target / "INDEX.md"
    if not index_md.exists():
        index_md.write_text(
            f"---\n"
            f'title: "Living System Catalog"\n'
            f"date: {today_str}\n"
            f"type: index\n"
            f"tags:\n"
            f"  - catalog\n"
            f"  - index\n"
            f'summary: "Master index of living systems architecture, projects, and operational ledgers."\n'
            f"---\n\n"
            f"# 🏛️ Living System Catalog\n\n"
            f"## 📌 Overview\n"
            f"Central catalog and index for systems architecture, service contracts, and operational ledgers.\n\n"
            f"## 🗺️ Maps of Content (MOCs)\n"
            f"- [[20-Projects/Projects-MOC|Projects MOC]]\n"
            f"- [[40-Systems/Systems-MOC|Systems MOC]]\n"
            f"- [[01-Daily/Daily-MOC|Daily MOC]]\n"
            f"- [[AGENTS|System Protocols & Invariants]]\n"
            f"- [[OPERATOR|Operator Profile]]\n",
            encoding="utf-8",
        )

    print(f"✅ Initialized fresh Akatsuki living memory vault at: {target}")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="akatsuki",
        description="Universal CLI and MCP Gateway for the akatsuki Agent Memory Substrate",
    )
    parser.add_argument(
        "--vault",
        "-V",
        dest="vault_path",
        help="Path to Akatsuki vault (defaults to $AKATSUKI_VAULT or auto-discovered)",
    )
    subparsers = parser.add_subparsers(dest="command")

    # init
    p_init = subparsers.add_parser("init", help="Bootstrap a new Akatsuki living memory vault")
    p_init.add_argument("path", nargs="?", default=".", help="Target directory to initialize vault in (default: .)")

    # search
    p_search = subparsers.add_parser("search", help="Search notes in akatsuki")
    p_search.add_argument("query", help="Keyword, topic, or exact phrase")
    p_search.add_argument(
        "--domain",
        "-d",
        choices=DOMAIN_DIRS,
    )
    p_search.add_argument("--limit", "-n", type=int, default=10)
    p_search.add_argument(
        "--mode",
        "-m",
        choices=["hybrid", "bm25", "vector"],
        default="hybrid",
        help="Retrieval mode: hybrid (BM25 + Vector, default), bm25, or vector",
    )
    p_search.add_argument(
        "--with-graph", "-g", action="store_true", help="Attach 1-hop graph relations and boundary sinks"
    )
    p_search.add_argument(
        "--compact", action="store_true", help="Dense token-optimized output format for agent prompts"
    )
    p_search.add_argument("--json", action="store_true", help="Output results as JSON payload")

    # read / cat
    p_read = subparsers.add_parser("read", aliases=["cat"], help="Read an akatsuki note or section")
    p_read.add_argument("note", help="Note title, stem, or relative path")
    p_read.add_argument("--section", "-s", help="Specific heading or section to extract")
    p_read.add_argument("--budget", "-b", type=int, help="Token budget packing (e.g. 200)")
    p_read.add_argument("--toc", action="store_true", help="Print table of contents outline")
    p_read.add_argument("--json", action="store_true", help="Output content as JSON payload")

    # contract
    p_contract = subparsers.add_parser("contract", help="Extract machine boundary contract from note")
    p_contract.add_argument("note", help="Note title, stem, or relative path")
    p_contract.add_argument("--json", action="store_true", help="Output contract as JSON object instead of YAML")

    # get
    p_get = subparsers.add_parser("get", help="O(1) exact property getter")
    p_get.add_argument("keypath", help="Keypath (e.g. services.api.ports, entities.service.repo)")
    p_get.add_argument("--json", action="store_true", help="Output keypath value as JSON payload")

    # query
    p_query = subparsers.add_parser("query", help="Execute read-only SQL against SQLite index")
    p_query.add_argument("sql", help="SQL query string (SELECT ...)")
    p_query.add_argument("--json", action="store_true", help="Output query results as JSON payload (default: JSON)")

    # blast
    p_blast = subparsers.add_parser("blast", help="Calculate architectural blast radius")
    p_blast.add_argument("target", help="Component, service, or system name")
    p_blast.add_argument("--json", action="store_true", help="Output structured JSON blast radius payload")

    # map
    p_map = subparsers.add_parser("map", help="Recursively map knowledge graph around a target note")
    p_map.add_argument("target", help="Component, note stem, or service name")
    p_map.add_argument("--depth", "-d", type=int, default=2, help="Traversal depth in hops (1-5, default: 2)")
    p_map.add_argument(
        "--direction",
        choices=["both", "down", "up"],
        default="both",
        help="Traversal direction: both, down (dependencies), or up (dependents)",
    )
    p_map.add_argument("--json", action="store_true", help="Output raw JSON graph structure")

    # test
    p_test = subparsers.add_parser("test", help="Execute machine-verifiable assertion blocks")
    p_test.add_argument("note", nargs="?", help="Optional note to filter tests")
    p_test.add_argument("--dry-run", "-n", action="store_true", help="Inspect test commands without executing them")
    p_test.add_argument("--json", action="store_true", help="Output test results as structured JSON")

    # set
    p_set = subparsers.add_parser("set", help="Surgically update a frontmatter key-value property")
    p_set.add_argument("note", help="Note title, stem, or relative path")
    p_set.add_argument("--key", "-k", required=True, help="Dot-separated keypath (e.g. status)")
    p_set.add_argument("--value", "-v", required=True, help="New value as string or JSON")
    p_set.add_argument("--json", action="store_true", help="Output result as JSON")

    # lint
    p_lint = subparsers.add_parser("lint", help="Validate vault notes against strict machine schemas")
    p_lint.add_argument("--json", action="store_true", help="Output lint report as JSON")

    # append
    p_append = subparsers.add_parser("append", help="Append content under a specific heading in a note")
    p_append.add_argument("note", help="Note title, stem, or relative path")
    p_append.add_argument("--heading", "-H", required=True, help="Heading under which to append content")
    p_append.add_argument("--content", "-c", help="Markdown content string to append")
    p_append.add_argument("--file", "-f", help="File containing markdown to append (or stdin)")
    p_append.add_argument("--json", action="store_true", help="Output result as JSON")

    # replace (Phase 3 Surgical Replace)
    p_replace = subparsers.add_parser("replace", help="Surgically replace content under a specific heading in a note")
    p_replace.add_argument("note", help="Note title, stem, or relative path")
    p_replace.add_argument("--heading", "-H", required=True, help="Heading section to replace")
    p_replace.add_argument("--content", "-c", help="New replacement markdown content")
    p_replace.add_argument("--file", "-f", help="File containing replacement content (or stdin)")
    p_replace.add_argument("--json", action="store_true", help="Output result as JSON")

    # list / ls
    p_list = subparsers.add_parser("list", aliases=["ls"], help="List notes in akatsuki")
    p_list.add_argument(
        "--domain",
        "-d",
        choices=DOMAIN_DIRS,
    )
    p_list.add_argument("--json", action="store_true", help="Output note catalog as JSON")

    # write
    p_write = subparsers.add_parser("write", help="Write a note, config, or script to akatsuki")
    p_write.add_argument("path", help="Relative path inside vault (e.g. 20-Projects/app.md, 50-Configs/traefik.yml)")
    p_write.add_argument("--content", "-c", help="Direct content to write (defaults to --file or stdin)")
    p_write.add_argument("--file", "-f", help="Source file to read content from (defaults to stdin)")
    p_write.add_argument("--overwrite", action="store_true", help="Overwrite if exists")
    p_write.add_argument(
        "--raw", action="store_true", help="Write raw content without forcing .md extension or frontmatter enforcement"
    )
    p_write.add_argument("--json", action="store_true", help="Output result as JSON")

    # services
    p_services = subparsers.add_parser("services", help="Print active services catalog and container allocations")
    p_services.add_argument("--json", action="store_true", help="Output services as structured JSON (default)")

    # projects
    p_projects = subparsers.add_parser("projects", help="Print active projects inventory")
    p_projects.add_argument("--json", action="store_true", help="Output projects as structured JSON (default)")

    # daily
    p_daily = subparsers.add_parser("daily", help="Print today's or specified daily note")
    p_daily.add_argument("date", nargs="?", help="YYYY-MM-DD date (defaults to today)")
    p_daily.add_argument("--json", action="store_true", help="Output daily note as structured JSON")

    # log
    p_log = subparsers.add_parser("log", help="Deposit a work log entry into today's note")
    p_log.add_argument("--project", "-p", default="", help="Project or repo name")
    p_log.add_argument("--summary", "-s", required=True, help="Punchy telegraphic summary (<280 chars soft limit)")
    p_log.add_argument("--device", "-d", default=None, help="Device/hostname identifier (defaults to current host)")
    p_log.add_argument("--json", action="store_true", help="Output confirmation as JSON")

    # verify
    p_verify = subparsers.add_parser("verify", help="Verify wikilinks and graph closure across vault")
    p_verify.add_argument("--json", action="store_true", help="Output verification results as JSON")

    # reconcile
    p_reconcile = subparsers.add_parser("reconcile", help="Auto-reconcile unindexed notes and strict YAML across vault")
    p_reconcile.add_argument(
        "--dry-run", "-n", action="store_true", help="Simulate reconciliation without writing changes"
    )
    p_reconcile.add_argument("--json", action="store_true", help="Output reconciliation actions as JSON")

    # mcp
    subparsers.add_parser("mcp", help="Run as Model Context Protocol (MCP) server on stdio")

    return parser
