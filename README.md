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

## Deployment troubleshooting

If the server starts but clients cannot connect, check these common self-hosting issues before changing the RustDesk client configuration:

- **Open all required ports on the host firewall, cloud security group, and NAT/router.** A standard OSS deployment needs `21115/tcp`, `21116/tcp`, `21116/udp`, and `21117/tcp`. If you enable the optional web client or web socket endpoints, also publish `21118/tcp` for `hbbs` and `21119/tcp` for `hbbr`.
- **Do not forget UDP on port `21116`.** Docker and many cloud firewalls require a separate UDP rule; opening only `21116/tcp` can leave discovery and direct connection setup failing.
- **For Docker Compose, publish the same ports that the containers listen on.** The sample `docker-compose.yml` maps `21115:21115`, `21116:21116`, `21116:21116/udp`, `21117:21117`, `21118:21118`, and `21119:21119`. If you change the left-hand host ports, update client/server settings accordingly.
- **Use a reachable relay address with `hbbs -r`.** Replace `rustdesk.example.com:21117` with a DNS name or public IP that clients can reach from the internet or your private network. Do not use `localhost` unless every client is on the same machine.
- **Keep `hbbs` and `hbbr` on shared persistent storage for key files.** In Docker Compose the `./data:/root` volume is used by both services so generated keys and IDs survive container restarts.
- **Do not delete or regenerate keys unintentionally.** Removing the mapped data directory creates a new key pair; clients configured with the previous key may fail until their server key/configuration is updated.
- **Check the key path when moving between bare-metal and containers.** Container examples store data under `/root` inside the container; host installs may use the working directory or a service-specific directory. Make sure the service user can read the existing key files.
- **Verify the containers are healthy before debugging clients.** Run `docker compose ps` and `docker compose logs hbbs hbbr` to confirm both services are running and that `hbbs` reports the relay address you expect.
- **Test reachability from outside the server.** A port can appear open locally while still being blocked by a cloud firewall or router. Test from another network for each published TCP port, and separately confirm that UDP `21116` is allowed.
- **Avoid mixing multiple RustDesk server instances behind the same address unless keys and relay settings match.** Clients may connect to one instance for ID lookup and another for relay, which can look like intermittent connection failures.

Quick checks:

1. Confirm the DNS name or public IP in the RustDesk client matches the address passed to `hbbs -r`.
2. Confirm `hbbr` is listening on the relay port (`21117/tcp` by default) and that clients can reach that same port externally.
3. Confirm Docker Compose was started from the directory containing the intended `./data` directory, especially after moving the compose file.
4. Confirm any reverse proxy or load balancer is passing raw TCP/UDP traffic; HTTP-only proxy rules are not sufficient for the standard server ports.
5. Confirm host firewalls such as `ufw`, `firewalld`, Windows Firewall, cloud security groups, and router port-forwarding rules all agree on the same ports.
6. After changing keys, relay host, or published ports, restart both `hbbs` and `hbbr`, then reconnect a client with the updated server settings.
