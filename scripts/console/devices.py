#!/usr/bin/env python3
import argparse
import sys

from common import find_user_id, get_peers, get_token, json_print, print_env_template, request


def main() -> int:
    parser = argparse.ArgumentParser(description="RustDesk console device manager")
    sub = parser.add_subparsers(dest="cmd", required=True)

    sub.add_parser("env", help="print env template")
    sub.add_parser("view", help="list devices")

    p_assign = sub.add_parser("assign", help="assign user to a device")
    p_assign.add_argument("user")
    p_assign.add_argument("peer_id")

    p_enable = sub.add_parser("enable", help="enable device")
    p_enable.add_argument("peer_id")

    p_disable = sub.add_parser("disable", help="disable device")
    p_disable.add_argument("peer_id")

    p_delete = sub.add_parser("delete", help="delete device")
    p_delete.add_argument("peer_id")

    args = parser.parse_args()

    if args.cmd == "env":
        print_env_template()
        return 0

    token = get_token()

    if args.cmd == "view":
        json_print(get_peers(token))
        return 0

    if args.cmd == "assign":
        user_id = find_user_id(token, args.user)
        json_print(request(f"/api/users/{user_id}/peers/{args.peer_id}", method="POST", token=token))
        return 0

    if args.cmd == "enable":
        json_print(request(f"/api/peers/{args.peer_id}/enable", method="POST", token=token))
        return 0

    if args.cmd == "disable":
        json_print(request(f"/api/peers/{args.peer_id}/disable", method="POST", token=token))
        return 0

    if args.cmd == "delete":
        json_print(request(f"/api/peers/{args.peer_id}", method="DELETE", token=token))
        return 0

    parser.print_help()
    return 2


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as err:
        print(f"ERROR: {err}", file=sys.stderr)
        raise SystemExit(1)
