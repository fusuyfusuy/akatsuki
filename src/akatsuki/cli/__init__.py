"""CLI entry point and command router for Akatsuki."""

from pathlib import Path

import akatsuki.storage as storage
from akatsuki.cli.commands import (
    cli_append,
    cli_blast,
    cli_contract,
    cli_daily,
    cli_get,
    cli_lint,
    cli_list,
    cli_log,
    cli_map,
    cli_projects,
    cli_query,
    cli_read,
    cli_reconcile,
    cli_replace,
    cli_search,
    cli_services,
    cli_set,
    cli_test,
    cli_verify,
    cli_write,
)
from akatsuki.cli.parser import build_parser, cli_init
from akatsuki.mcp.server import run_mcp_server


def main():
    parser = build_parser()
    args = parser.parse_args()

    if getattr(args, "vault_path", None):
        override_path = Path(args.vault_path).expanduser().resolve()
        storage.CURRENT_VAULT_OVERRIDE = override_path
        try:
            import akatsuki.core as core

            core.CURRENT_VAULT_OVERRIDE = override_path
        except (ImportError, AttributeError):
            pass

    if args.command == "init":
        cli_init(args)
    elif args.command == "search":
        cli_search(args)
    elif args.command in ("read", "cat"):
        cli_read(args)
    elif args.command == "contract":
        cli_contract(args)
    elif args.command == "get":
        cli_get(args)
    elif args.command == "query":
        cli_query(args)
    elif args.command == "blast":
        cli_blast(args)
    elif args.command == "map":
        cli_map(args)
    elif args.command == "test":
        cli_test(args)
    elif args.command == "set":
        cli_set(args)
    elif args.command == "lint":
        cli_lint(args)
    elif args.command == "append":
        cli_append(args)
    elif args.command == "replace":
        cli_replace(args)
    elif args.command in ("list", "ls"):
        cli_list(args)
    elif args.command == "write":
        cli_write(args)
    elif args.command == "services":
        cli_services(args)
    elif args.command == "projects":
        cli_projects(args)
    elif args.command == "daily":
        cli_daily(args)
    elif args.command == "log":
        cli_log(args)
    elif args.command == "verify":
        cli_verify(args)
    elif args.command == "reconcile":
        cli_reconcile(args)
    elif args.command == "mcp":
        run_mcp_server()
    else:
        parser.print_help()


if __name__ == "__main__":
    main()
