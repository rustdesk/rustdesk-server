#!/usr/bin/env sh
# Starts hbbs with the IP whitelist check enabled.
# Any extra arguments are forwarded to hbbs.

set -e

cd "$(dirname "$0")"

DEFAULT_URL='http://127.0.0.1:3000/servicos/rustdesk/liberacoes/{ip}'
export IP_WHITELIST_URL="${IP_WHITELIST_URL:-$DEFAULT_URL}"
export IP_WHITELIST_TIMEOUT_MS="${IP_WHITELIST_TIMEOUT_MS:-3000}"
export IP_WHITELIST_CACHE_SEC="${IP_WHITELIST_CACHE_SEC:-60}"
export IP_WHITELIST_FAIL_OPEN="${IP_WHITELIST_FAIL_OPEN:-N}"

BIN=""
for candidate in target/release/hbbs target/release/hbbs.exe target/debug/hbbs target/debug/hbbs.exe; do
    if [ -x "$candidate" ] || [ -f "$candidate" ]; then
        BIN="$candidate"
        break
    fi
done

if [ -z "$BIN" ]; then
    echo "hbbs não encontrado; rode 'cargo build --bin hbbs' antes." >&2
    exit 1
fi

echo "Usando $BIN"
echo "IP_WHITELIST_URL=$IP_WHITELIST_URL"
exec "$BIN" "$@"
