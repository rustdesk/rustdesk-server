# Configuration & Environment Variables

This document is the single reference for every option that the open‑source
RustDesk server binaries (`hbbs`, `hbbr`) understand: command‑line flags,
environment variables, and configuration files.

> **TL;DR** — For most people the command‑line flags shown by `hbbs --help` /
> `hbbr --help` are all you need. Environment variables are an alternative way to
> set the same options, plus a handful of extra tuning knobs that have no flag.

---

## How configuration is loaded

Both servers read their configuration from three sources. For **`hbbs`** the
order of precedence, from highest to lowest, is:

1. **Command‑line flag** (e.g. `-p 21116`, `-k mykey`)
2. **`--config <file>`** — an INI file passed with `-c`/`--config`
3. **`.env`** — an INI file named `.env` in the working directory
4. **Inherited process environment** — variables exported before launch

A value set by a higher source overrides the same value from a lower one. Under
the hood every source is turned into a process environment variable, and the
code then reads that variable — so "flag", "config file" and "env var" are just
three ways to set the same thing.

For **`hbbr`** the precedence is: **flag** (`-p`, `-k`) → **`.env`** →
**inherited environment**.

### ⚠️ The `.env` naming gotcha (read this)

`hbbs` and `hbbr` do **not** parse `.env` the same way:

| | Key written in `.env` | Variable the code sees |
|---|---|---|
| **`hbbs`** | `relay_servers` **or** `relay-servers` | `RELAY-SERVERS` (upper‑cased, `_`→`-`) |
| **`hbbr`** | `downgrade_threshold` | `DOWNGRADE_THRESHOLD` (used verbatim) |

* `hbbs` rewrites every `.env`/`--config` key to **UPPERCASE** and replaces
  underscores with dashes. So a key with an underscore in `.env` (for example
  `DB_URL` or `TEST_HBBS`) becomes `DB-URL` / `TEST-HBBS` and will **not** match
  the `DB_URL` / `TEST_HBBS` the code looks for. Those "direct" variables
  (marked 🅴 in the tables below) can therefore **only** be set as a real
  environment variable, not through the `hbbs` `.env`/`--config` file.
* `hbbr` uses `.env` keys verbatim, so `.env` works for all of its variables.

**Recommendation:**
* Set the documented flag options via the **command line** or the **`.env`
  file** (using the dashed lowercase name, e.g. `relay-servers`).
* Set the 🅴 "direct" tuning variables as **real environment variables**
  (docker‑compose `environment:`, systemd `Environment=`, or `export`).

Multi‑word options such as `relay-servers` have an internal env‑var name that
contains a dash (`RELAY-SERVERS`). Most shells cannot `export RELAY-SERVERS=…`,
so for those prefer the CLI flag or the `.env` file. Single‑word options
(`PORT`, `KEY`, `MASK`, `SERIAL`, `RMEM`) are ordinary identifiers and work fine
as exported environment variables.

---

## `hbbs` — ID / rendezvous server

| Variable | CLI flag | Default | Description |
|---|---|---|---|
| `KEY` | `-k`, `--key` | `-` | Public key clients must use, or a base64 secret key, or `-` / `_` to auto‑generate a key pair (`id_ed25519`, `id_ed25519.pub`). Use `_` to require encryption (see [Keys](#keys-and-encryption)). |
| `PORT` | `-p`, `--port` | `21116` | Main TCP/UDP listening port. `hbbs` also binds `PORT-1` (NAT type test) and `PORT+2` (WebSocket). |
| `RELAY-SERVERS` | `-r`, `--relay-servers` | *(empty)* | Default relay server(s) handed to clients, comma‑separated `host` or `host:port`. Usually your public IP / domain. |
| `RENDEZVOUS-SERVERS` | `-R`, `--rendezvous-servers` | *(empty)* | Peer rendezvous servers to forward to, comma‑separated. For multi‑server setups; leave empty for a single server. |
| `MASK` | `--mask` | *(none)* | CIDR that marks a client as "LAN", e.g. `192.168.0.0/16`. When set, LAN peers get `LOCAL-IP` instead of their public address. |
| `LOCAL-IP` | *(none — env/`.env` only)* | auto‑detected | LAN address advertised to peers matched by `MASK`. Defaults to the machine's primary local IP. |
| `SERIAL` | `-s`, `--serial` | `0` | Config update serial. Bump it to push updated relay/rendezvous lists to clients. |
| `RMEM` | `-M`, `--rmem` | `0` (system default) | UDP receive‑buffer size in bytes. Raise the OS limit first: `sudo sysctl -w net.core.rmem_max=52428800`. |
| `SOFTWARE-URL` | `-u`, `--software-url` | *(empty)* | Download URL of the newest RustDesk client; the version is parsed from it and offered to clients. |
| *(config file)* | `-c`, `--config` | *(none)* | Path to an extra INI config file (see precedence above). |
| `TEST_HBBS` 🅴 | *(none)* | *(auto)* | UDP self‑test target checked at start‑up. Set to `no` to skip the check (useful behind some NATs/proxies), or to an explicit `host:port`. |
| `ALWAYS_USE_RELAY` 🅴 | *(none)* | `N` | `Y` forces every session through a relay (disables direct/hole‑punched connections). Also toggleable at runtime via the `rustdesk-utils` / console `always-use-relay` command. |
| `DB_URL` 🅴 | *(none)* | `./db_v2.sqlite3` | Path/URL of the SQLite database file. See [Database](#database). |
| `MAX_DATABASE_CONNECTIONS` 🅴 | *(none)* | `1` | Size of the SQLite connection pool. |

🅴 = read directly from the environment; **cannot** be set through the `hbbs`
`.env`/`--config` file (see the gotcha above).

> `PORT_FOR_API` / `KEY_FOR_API` are only used by RustDesk Server **Pro** and its
> API; they have no effect in the open‑source server.

---

## `hbbr` — relay server

| Variable | CLI flag | Default | Description |
|---|---|---|---|
| `KEY` | `-k`, `--key` | *(empty)* | Same meaning as for `hbbs`. Must match the key `hbbs` uses. `-` / `_` auto‑generate / require encryption. |
| `PORT` | `-p`, `--port` | `21117` | Relay listening port. `hbbr` also binds `PORT+2` for WebSocket relay. **Note:** when set via the `PORT` env var (not `-p`), `hbbr` listens on `PORT + 1`, so a shared `PORT=21116` makes `hbbs`=21116 and `hbbr`=21117. |

### Relay bandwidth / QoS (all 🅴, set as environment variables)

These have no CLI flag and can also be changed at runtime through the `hbbr`
interactive console (`ba`, `tb`, `sb`, `ls`, `dt`, `t`, …; type `h` for help).

| Variable | Default | Unit | Description |
|---|---|---|---|
| `SINGLE_BANDWIDTH` | `128` | Mb/s | Max bandwidth a single relay connection may use. |
| `TOTAL_BANDWIDTH` | `1024` | Mb/s | Max aggregate bandwidth across all relay connections before throttling kicks in. |
| `LIMIT_SPEED` | `32` | Mb/s | Reduced per‑connection speed applied to "heavy" connections once the relay is congested. |
| `DOWNGRADE_THRESHOLD` | `0.66` | ratio (0–1) | Load ratio above which a connection is treated as heavy and eligible for downgrade. |
| `DOWNGRADE_START_CHECK` | `1800` | seconds | How long a connection must run before it is evaluated for downgrade. |

`hbbr` reads its `.env` verbatim, so these may also be placed in `.env`
(e.g. `SINGLE_BANDWIDTH=256`).

### Blocklists / blacklists (files, not env vars)

`hbbr` reads two optional files from its working directory at start‑up:

* **`blacklist.txt`** — IPs that are **bandwidth‑limited** (one IP per line;
  anything after the first space on a line is ignored).
* **`blocklist.txt`** — IPs that are **refused** outright.

Both can also be edited live from the `hbbr` console (`ba`/`br`, `Ba`/`Br`).

---

## Database

At runtime the database location comes from **`DB_URL`** (default
`./db_v2.sqlite3`). If unset, `hbbs` creates the SQLite file in its working
directory.

> **Do not confuse `DB_URL` with `DATABASE_URL`.** The `DATABASE_URL` entry in
> the repository's `.env` is used **only at compile time** by `sqlx` to check SQL
> queries; it is **not** read by the running server. Setting `DATABASE_URL` on a
> running server has no effect — use `DB_URL`.

---

## Logging

Both binaries use `flexi_logger`, which honours the standard **`RUST_LOG`**
environment variable (default level `info`).

```bash
RUST_LOG=debug hbbs -r example.com
```

---

## Keys and encryption

The `KEY` / `-k` value can be:

* a **public key** string — clients must present the matching key;
* a **base64‑encoded 64‑byte secret key** — the server derives the public key
  from it;
* **`-`, `_`, or empty** — the server auto‑generates a key pair on first start,
  writing `id_ed25519` (private) and `id_ed25519.pub` (public) to the working
  directory, and prints the public key in the log.

By convention `-k _` is used to run an **encryption‑only** server (the official
Docker image exposes this as `ENCRYPTED_ONLY=1`). `hbbs` and `hbbr` must be
started with the **same** key.

To supply your own key pair, place `id_ed25519` and `id_ed25519.pub` next to the
binaries (or in `/data` for Docker) before first start.

---

## Docker image variables

The official image (`rustdesk/rustdesk-server`) wraps the binaries with an
s6 supervisor and adds a few convenience variables handled by the entrypoint,
**not** by `hbbs`/`hbbr` directly:

| Variable | Default | Description |
|---|---|---|
| `RELAY` | `relay.example.com` | Passed to `hbbs` as `-r $RELAY` (your public address). |
| `ENCRYPTED_ONLY` | `0` | `1` adds `-k _` to both servers, forcing encryption. |
| `KEY_PUB` | *(unset)* | If set, written to `/data/id_ed25519.pub` on first start. |
| `KEY_PRIV` | *(unset)* | If set, written to `/data/id_ed25519` on first start. Provide **both** `KEY_PUB` and `KEY_PRIV`, or neither. |

Any variable from the tables above can also be passed straight through the
container's environment (e.g. `-e ALWAYS_USE_RELAY=Y`, `-e RUST_LOG=debug`).

---

## Examples

### Command line

```bash
# ID server: relay clients to this host, LAN detection, force encryption
hbbs -r rustdesk.example.com:21117 --mask 192.168.0.0/16 -k _

# Relay server, same key
hbbr -k _
```

### `.env` file (working directory)

```ini
# Works for both binaries. Use dashed lowercase names for hbbs flag options.
relay-servers = rustdesk.example.com:21117
key = _
port = 21116
```

> Reminder: put the 🅴 variables (`DB_URL`, `TEST_HBBS`, `ALWAYS_USE_RELAY`,
> `MAX_DATABASE_CONNECTIONS`) in the real environment, not in the `hbbs` `.env`.

### docker-compose

```yaml
services:
  hbbs:
    image: rustdesk/rustdesk-server:latest
    command: hbbs -r rustdesk.example.com:21117
    environment:
      - ENCRYPTED_ONLY=1
      - ALWAYS_USE_RELAY=Y
      - RUST_LOG=info
    ports: ["21115:21115", "21116:21116", "21116:21116/udp", "21118:21118"]
    volumes: ["./data:/root"]
    restart: unless-stopped

  hbbr:
    image: rustdesk/rustdesk-server:latest
    command: hbbr
    environment:
      - ENCRYPTED_ONLY=1
      - SINGLE_BANDWIDTH=256
    ports: ["21117:21117", "21119:21119"]
    volumes: ["./data:/root"]
    restart: unless-stopped
```

### systemd

```ini
[Service]
Environment=ALWAYS_USE_RELAY=Y
Environment=RUST_LOG=info
ExecStart=/usr/bin/hbbs -r rustdesk.example.com:21117 -k _
```

---

## Port reference

| Port | Proto | Server | Purpose |
|---|---|---|---|
| 21115 | TCP | hbbs | NAT type test (`PORT-1`) |
| 21116 | TCP + UDP | hbbs | ID registration / rendezvous / hole punching (`PORT`) |
| 21117 | TCP | hbbr | Relay (`hbbr PORT`) |
| 21118 | TCP | hbbs | WebSocket rendezvous (`PORT+2`) |
| 21119 | TCP | hbbr | WebSocket relay (`hbbr PORT+2`) |

Ports 21118/21119 are only needed for the web client; you can omit them
otherwise.
