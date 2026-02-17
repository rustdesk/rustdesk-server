#!/usr/bin/env python3
import argparse
import sys

from common import find_user_id, get_token, json_print, print_env_template, request


def main() -> int:
    parser = argparse.ArgumentParser(description="RustDesk console user manager")
    sub = parser.add_subparsers(dest="cmd", required=True)

    sub.add_parser("env", help="print env template")
    sub.add_parser("view", help="list users")

    p_new = sub.add_parser("new", help="create user")
    p_new.add_argument("username")
    p_new.add_argument("password")
    p_new.add_argument("role", nargs="?", default="user", choices=["user", "admin"])

    p_enable = sub.add_parser("enable", help="enable user")
    p_enable.add_argument("user")

    p_disable = sub.add_parser("disable", help="disable user")
    p_disable.add_argument("user")

    p_delete = sub.add_parser("delete", help="delete user")
    p_delete.add_argument("user")

    args = parser.parse_args()

    if args.cmd == "env":
        print_env_template()
        return 0

    token = get_token()

    if args.cmd == "view":
        json_print(request("/api/users", token=token))
        return 0

    if args.cmd == "new":
        json_print(request("/api/users", method="POST", token=token, data={
            "username": args.username,
            "password": args.password,
            "role": args.role,
        }))
        return 0

    user_id = find_user_id(token, args.user)

    if args.cmd == "enable":
        json_print(request(f"/api/users/{user_id}/enable", method="POST", token=token))
        return 0

    if args.cmd == "disable":
        json_print(request(f"/api/users/{user_id}/disable", method="POST", token=token))
        return 0

    if args.cmd == "delete":
        json_print(request(f"/api/users/{user_id}", method="DELETE", token=token))
        return 0

    parser.print_help()
    return 2


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as err:
        print(f"ERROR: {err}", file=sys.stderr)
        raise SystemExit(1)

