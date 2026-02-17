# RustDesk Server Program

[![build](https://github.com/rustdesk/rustdesk-server/actions/workflows/build.yaml/badge.svg)](https://github.com/rustdesk/rustdesk-server/actions/workflows/build.yaml)

[**Download**](https://github.com/rustdesk/rustdesk-server/releases)

[**Manual**](https://rustdesk.com/docs/en/self-host/)

[**FAQ**](https://github.com/rustdesk/rustdesk/wiki/FAQ)

[**How to migrate OSS to Pro**](https://rustdesk.com/docs/en/self-host/rustdesk-server-pro/installscript/#convert-from-open-source)

Self-host your own RustDesk server, it is free and open source.

## How to build manually

```bash
cargo build --release
```

Three executables will be generated in target/release.

- hbbs - RustDesk ID/Rendezvous server
- hbbr - RustDesk relay server
- rustdesk-utils - RustDesk CLI utilities

You can find updated binaries on the [Releases](https://github.com/rustdesk/rustdesk-server/releases) page.

If you want extra features, [RustDesk Server Pro](https://rustdesk.com/pricing.html) might suit you better.

If you want to develop your own server, [rustdesk-server-demo](https://github.com/rustdesk/rustdesk-server-demo) might be a better and simpler start for you than this repo.

## Installation

Please follow this [doc](https://rustdesk.com/docs/en/self-host/rustdesk-server-oss/)

## Console/API (custom patch)

This workspace includes a built-in admin console/API on `--api-port` (default `hbbs_port - 2`).

- Login API: `POST /api/login`
- Current user: `GET /api/currentUser`
- Users: `GET/POST /api/users`, `POST /api/users/:id/enable|disable`, `DELETE /api/users/:id`
- Devices: `GET /api/peers`, `POST /api/peers/:id/enable|disable`, `DELETE /api/peers/:id`
- User-device ACL: `GET /api/users/:id/peers`, `POST/DELETE /api/users/:id/peers/:peer_id`
- Connection audits: `GET /api/audits/conn`

For command-line operations, helper scripts are added in `scripts/console/`:

- `users.py`
- `devices.py`
- `audits.py`

Run any script with `env` sub-command to print required environment variables.
