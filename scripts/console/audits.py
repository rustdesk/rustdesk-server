#!/usr/bin/env python3
import argparse
import sys

from common import get_token, json_print, print_env_template, request


def main() -> int:
    parser = argparse.ArgumentParser(description="RustDesk console audit manager")
    sub = parser.add_subparsers(dest="cmd", required=True)

    sub.add_parser("env", help="print env template")

    p_view = sub.add_parser("view-conn", help="view recent connection audits")
    p_view.add_argument("--offset", type=int, default=0)
    p_view.add_argument("--limit", type=int, default=50)

    args = parser.parse_args()

    if args.cmd == "env":
        print_env_template()
        return 0

    token = get_token()

    if args.cmd == "view-conn":
        path = f"/api/audits/conn?offset={max(args.offset, 0)}&limit={max(args.limit, 1)}"
        json_print(request(path, token=token))
        return 0

    parser.print_help()
    return 2


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as err:
        print(f"ERROR: {err}", file=sys.stderr)
        raise SystemExit(1)
