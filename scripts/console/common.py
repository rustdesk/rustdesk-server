#!/usr/bin/env python3
import json
import os
import urllib.error
import urllib.request
from typing import Any, Dict, Optional


def getenv(name: str, default: str = "") -> str:
    return os.environ.get(name, default)


def print_env_template() -> None:
    print("# export/set these env vars before using scripts")
    print("RUSTDESK_API=http://127.0.0.1:21114")
    print("RUSTDESK_USERNAME=admin")
    print("RUSTDESK_PASSWORD=admin123456")
    print("RUSTDESK_TOKEN=<optional, script can auto-login>")


def _base_url() -> str:
    return getenv("RUSTDESK_API", "http://127.0.0.1:21114").rstrip("/")


def _request(
    path: str,
    method: str = "GET",
    data: Optional[Dict[str, Any]] = None,
    token: Optional[str] = None,
):
    url = _base_url() + path
    headers = {"Content-Type": "application/json"}
    if token:
        headers["token"] = token
    body = None if data is None else json.dumps(data).encode("utf-8")
    req = urllib.request.Request(url, data=body, method=method, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            raw = resp.read().decode("utf-8")
            return json.loads(raw) if raw else {}
    except urllib.error.HTTPError as err:
        try:
            payload = err.read().decode("utf-8")
        except Exception:
            payload = ""
        raise RuntimeError(f"HTTP {err.code} {err.reason}: {payload}") from err


def get_token() -> str:
    token = getenv("RUSTDESK_TOKEN")
    if token:
        return token
    username = getenv("RUSTDESK_USERNAME", "admin")
    password = getenv("RUSTDESK_PASSWORD", "admin123456")
    data = _request("/api/login", method="POST", data={"username": username, "password": password})
    token = data.get("token", "")
    if not token:
        raise RuntimeError("failed to obtain token from /api/login")
    return token


def json_print(data) -> None:
    print(json.dumps(data, indent=2, ensure_ascii=False))


def get_users(token: str):
    return _request("/api/users", token=token)


def find_user_id(token: str, user: str) -> int:
    users = get_users(token)
    if user.isdigit():
        return int(user)
    for u in users:
        if u.get("username") == user:
            return int(u["id"])
    raise RuntimeError(f"user not found: {user}")


def get_peers(token: str):
    return _request("/api/peers", token=token)


def request(path: str, method: str = "GET", data: Optional[Dict[str, Any]] = None, token: Optional[str] = None):
    return _request(path, method=method, data=data, token=token)
