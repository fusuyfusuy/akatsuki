"""Command handlers for Akatsuki CLI with dual terminal and JSON output formatting."""

import datetime
import json
import re
import sys
from pathlib import Path

from akatsuki.graph import calculate_blast_radius, traverse_graph
from akatsuki.index import execute_sql_query, fts_db_context, sync_fts_index
from akatsuki.markdown import apply_token_budget, slice_markdown_section
from akatsuki.mutations import (
    append_section_to_note,
    append_work_log,
    list_notes_in_vault,
    replace_section_in_note,
    set_note_property,
    write_note,
)
from akatsuki.search import (
    extract_note_contract,
    get_keypath,
    search_vault,
)
from akatsuki.storage import get_vault, resolve_note_file
from akatsuki.verify import lint_vault, reconcile_vault, run_verification_tests, verify_links


def cli_search(args):
    vault = get_vault()
    with_graph = getattr(args, "with_graph", False)
    mode = getattr(args, "mode", "hybrid")
    as_json = getattr(args, "json", False)
    compact = getattr(args, "compact", False)

    results = search_vault(
        vault,
        args.query,
        domain=args.domain,
        limit=args.limit,
        with_graph=with_graph,
        mode=mode,
    )

    if as_json:
        print(json.dumps(results, indent=2, default=str))
        return

    if not results:
        print(f"No notes found matching '{args.query}'.")
        return

    if compact:
        for r in results:
            graph_info = ""
            if r.get("graph"):
                g = r["graph"]
                up = f"up:[{','.join(g.get('upstream', []))}]" if g.get("upstream") else ""
                down = f"down:[{','.join(g.get('downstream', []))}]" if g.get("downstream") else ""
                graph_info = f" | {up} {down}".strip()
            snippet = f" | {r['snippet']}" if r.get("snippet") else ""
            print(f"[{r['stem']}] {r['title']} ({r['rel_path']}) score:{r['score']}{snippet}{graph_info}")
        return

    print(f"Found {len(results)} note(s) matching '{args.query}' [{mode.upper()}]:\n")
    for r in results:
        print(f"📄 {r['title']} ({r['rel_path']}) [Score: {r['score']}]")
        if r.get("breadcrumb"):
            print(f"   Section: {r['breadcrumb']}")
        if r.get("summary"):
            print(f"   Summary: {r['summary']}")
        if r.get("snippet"):
            print(f"   Excerpt: {r['snippet']}")
        if r.get("graph"):
            g = r["graph"]
            print("   Connected Graph:")
            if g.get("upstream"):
                print(f"     - Upstream: {', '.join(g['upstream'])}")
            if g.get("downstream"):
                print(f"     - Downstream: {', '.join(g['downstream'])}")
            if g.get("services"):
                print(f"     - Boundary: {', '.join(g['services'])}")
        print()


def cli_read(args):
    vault = get_vault()
    as_json = getattr(args, "json", False)
    note_file = resolve_note_file(vault, args.note)
    if not note_file:
        if as_json:
            print(json.dumps({"error": f"Note '{args.note}' not found in akatsuki vault."}, indent=2), file=sys.stderr)
        else:
            print(f"Error: Note '{args.note}' not found in akatsuki vault.", file=sys.stderr)
        sys.exit(1)

    content = note_file.read_text(encoding="utf-8")
    if getattr(args, "toc", False):
        _, toc_lines = slice_markdown_section(content, "__toc__")
        if as_json:
            print(json.dumps({"note": note_file.stem, "toc": toc_lines}, indent=2))
        else:
            print(f"Table of Contents for {note_file.stem}:\n")
            for line in toc_lines:
                print(f"  {line}")
        return

    section = getattr(args, "section", None)
    if section:
        sliced, toc_lines = slice_markdown_section(content, section)
        if sliced is None:
            if as_json:
                print(
                    json.dumps(
                        {"error": f"Section '{section}' not found in '{note_file.name}'.", "available": toc_lines},
                        indent=2,
                    ),
                    file=sys.stderr,
                )
            else:
                print(f"Section '{section}' not found in '{note_file.name}'.", file=sys.stderr)
                print("Available sections:\n" + "\n".join(f"  - {line}" for line in toc_lines), file=sys.stderr)
            sys.exit(1)
        content = sliced

    budget = getattr(args, "budget", None)
    if budget:
        content = apply_token_budget(content, budget)

    if as_json:
        print(
            json.dumps(
                {
                    "note": note_file.stem,
                    "rel_path": str(note_file.relative_to(vault)),
                    "section": section,
                    "content": content,
                },
                indent=2,
            )
        )
    else:
        print(content)


def cli_contract(args):
    vault = get_vault()
    as_json = getattr(args, "json", False)
    contract, is_err = extract_note_contract(vault, args.note, as_json=as_json)
    if is_err:
        print(contract, file=sys.stderr)
        sys.exit(1)
    print(contract)


def cli_get(args):
    vault = get_vault()
    val, is_err = get_keypath(vault, args.keypath)
    if is_err:
        print(val, file=sys.stderr)
        sys.exit(1)

    if getattr(args, "json", False):
        try:
            parsed = json.loads(val)
        except Exception:
            parsed = val
        print(json.dumps({"keypath": args.keypath, "value": parsed}, indent=2, default=str))
    else:
        print(val)


def cli_query(args):
    vault = get_vault()
    out, is_err = execute_sql_query(vault, args.sql)
    if is_err:
        print(out, file=sys.stderr)
        sys.exit(1)
    print(out)


def cli_blast(args):
    vault = get_vault()
    as_json = getattr(args, "json", False)
    out, is_err = calculate_blast_radius(vault, args.target, as_json=as_json)
    if is_err:
        print(out, file=sys.stderr)
        sys.exit(1)
    print(out)


def cli_map(args):
    vault = get_vault()
    text_out, is_err, json_data = traverse_graph(vault, args.target, depth=args.depth, direction=args.direction)
    if getattr(args, "json", False):
        print(json.dumps(json_data, indent=2))
    else:
        print(text_out)
    if is_err:
        sys.exit(1)


def cli_test(args):
    vault = get_vault()
    as_json = getattr(args, "json", False)
    dry_run = getattr(args, "dry_run", False)
    out, is_err = run_verification_tests(vault, note_filter=args.note, as_json=as_json, dry_run=dry_run)
    print(out)
    if is_err:
        sys.exit(1)


def cli_set(args):
    vault = get_vault()
    msg, is_err = set_note_property(vault, args.note, args.key, args.value)
    if is_err:
        print(msg, file=sys.stderr)
        sys.exit(1)

    if getattr(args, "json", False):
        print(
            json.dumps(
                {"success": True, "message": msg, "note": args.note, "key": args.key, "value": args.value},
                indent=2,
            )
        )
    else:
        print(msg)


def cli_lint(args):
    vault = get_vault()
    as_json = getattr(args, "json", False)
    out, is_err = lint_vault(vault, as_json=as_json)
    if is_err:
        print(out, file=sys.stderr)
        sys.exit(1)
    print(out)


def cli_append(args):
    vault = get_vault()
    content = args.content
    if not content:
        content = Path(args.file).read_text(encoding="utf-8") if args.file else sys.stdin.read()

    if not content or not content.strip():
        err_msg = "Error: No content provided to append."
        if getattr(args, "json", False):
            print(json.dumps({"error": err_msg}, indent=2), file=sys.stderr)
        else:
            print(err_msg, file=sys.stderr)
        sys.exit(1)

    msg, is_err = append_section_to_note(vault, args.note, args.heading, content)
    if is_err:
        print(msg, file=sys.stderr)
        sys.exit(1)

    if getattr(args, "json", False):
        print(
            json.dumps(
                {"status": "ok", "success": True, "message": msg, "note": args.note, "heading": args.heading},
                indent=2,
            )
        )
    else:
        print(msg)


def cli_replace(args):
    vault = get_vault()
    content = args.content
    if not content:
        content = Path(args.file).read_text(encoding="utf-8") if args.file else sys.stdin.read()

    if not content or not content.strip():
        err_msg = "Error: No replacement content provided."
        if getattr(args, "json", False):
            print(json.dumps({"error": err_msg}, indent=2), file=sys.stderr)
        else:
            print(err_msg, file=sys.stderr)
        sys.exit(1)

    msg, is_err = replace_section_in_note(vault, args.note, args.heading, content)
    if is_err:
        print(msg, file=sys.stderr)
        sys.exit(1)

    if getattr(args, "json", False):
        print(
            json.dumps(
                {"status": "ok", "success": True, "message": msg, "note": args.note, "heading": args.heading},
                indent=2,
            )
        )
    else:
        print(msg)


def cli_services(args):
    vault = get_vault()
    with fts_db_context(vault) as con:
        sync_fts_index(vault, con)
        cur = con.execute("SELECT name, container_prefix, ports, replicas, role, host, network FROM services")
        rows = [dict(r) for r in cur.fetchall()]
    if rows or getattr(args, "json", False):
        print(json.dumps(rows, indent=2, default=str))
        return
    catalog = vault / "40-Systems" / "Services-Catalog.md"
    if catalog.exists():
        print(catalog.read_text(encoding="utf-8"))


def cli_projects(args):
    vault = get_vault()
    with fts_db_context(vault) as con:
        sync_fts_index(vault, con)
        cur = con.execute(
            "SELECT stem, title, status, repo, host, network, summary FROM entities WHERE type = 'project'"
        )
        rows = [dict(r) for r in cur.fetchall()]
    if rows or getattr(args, "json", False):
        print(json.dumps(rows, indent=2, default=str))
        return
    moc = vault / "20-Projects" / "Projects-MOC.md"
    if not moc.exists():
        moc = vault / "INDEX.md"
    print(moc.read_text(encoding="utf-8"))


def cli_daily(args):
    vault = get_vault()
    as_json = getattr(args, "json", False)
    if args.date:
        target_date = args.date.strip()
        if not re.match(r"^\d{4}-\d{2}-\d{2}$", target_date):
            err_msg = f"Error: Invalid date format '{target_date}'. Expected YYYY-MM-DD."
            if as_json:
                print(json.dumps({"error": err_msg}, indent=2), file=sys.stderr)
            else:
                print(err_msg, file=sys.stderr)
            sys.exit(1)
    else:
        target_date = datetime.datetime.now().astimezone().strftime("%Y-%m-%d")

    note = vault / "01-Daily" / f"{target_date}.md"
    exists = note.exists()
    content = note.read_text(encoding="utf-8") if exists else ""

    if as_json:
        print(json.dumps({"date": target_date, "exists": exists, "content": content}, indent=2))
        return

    if not exists:
        print(f"Daily note for {target_date} does not exist yet.")
        return
    print(content)


def cli_log(args):
    vault = get_vault()
    device = getattr(args, "device", None)
    res = append_work_log(vault, args.project, args.summary, device=device)
    if getattr(args, "json", False):
        print(
            json.dumps(
                {
                    "status": "ok",
                    "success": True,
                    "message": res,
                    "project": args.project,
                    "summary": args.summary,
                    "device": device,
                },
                indent=2,
            )
        )
    else:
        print(res)


def cli_verify(args):
    vault = get_vault()
    as_json = getattr(args, "json", False)
    ok, broken = verify_links(vault)
    if not ok:
        if as_json:
            issues = [{"source": src, "issue": issue} for src, issue in broken]
            print(json.dumps({"passed": False, "total_issues": len(broken), "issues": issues}, indent=2), file=sys.stderr)
        else:
            print(f"FAILED: {len(broken)} link/graph issue(s) found in akatsuki:", file=sys.stderr)
            for src, issue in broken[:15]:
                print(f"  - In '{src}': {issue}", file=sys.stderr)
        sys.exit(1)

    if as_json:
        print(json.dumps({"passed": True, "issues": []}, indent=2))
    else:
        print("PASSED: All wikilinks and markdown links in akatsuki resolve cleanly (zero orphans).")


def cli_reconcile(args):
    vault = get_vault()
    as_json = getattr(args, "json", False)
    out, is_err = reconcile_vault(vault, dry_run=args.dry_run)
    if is_err:
        print(out, file=sys.stderr)
        sys.exit(1)

    if as_json:
        print(json.dumps({"status": "ok", "success": True, "dry_run": args.dry_run, "report": out}, indent=2))
    else:
        print(out)


def cli_list(args):
    vault = get_vault()
    notes = list_notes_in_vault(vault, domain=args.domain)
    if getattr(args, "json", False):
        print(json.dumps(notes, indent=2, default=str))
        return

    if not notes:
        print("No notes found.")
        return
    print(f"Notes in akatsuki ({len(notes)}):\n")
    for n in notes:
        print(f"• {n['stem']} [{n['type']}] ({n['rel_path']})")
        if n["summary"]:
            print(f"    {n['summary']}")


def cli_write(args):
    vault = get_vault()
    if getattr(args, "content", None) is not None:
        content = args.content
    elif args.file:
        content = Path(args.file).read_text(encoding="utf-8")
    else:
        content = sys.stdin.read()
    raw_flag = getattr(args, "raw", False)
    msg, is_err = write_note(vault, args.path, content, overwrite=args.overwrite, raw=raw_flag)
    if is_err:
        print(msg, file=sys.stderr)
        sys.exit(1)

    if getattr(args, "json", False):
        print(
            json.dumps(
                {
                    "status": "ok",
                    "success": True,
                    "message": msg,
                    "path": args.path,
                    "overwrite": args.overwrite,
                    "raw": raw_flag,
                },
                indent=2,
            )
        )
    else:
        print(msg)
