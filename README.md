# RustDesk Server Program

[**Download**](https://github.com/rustdesk/rustdesk-server/releases)

[**Manual**](https://rustdesk.com/docs/en/self-host/) 

[**FAQ**](https://github.com/rustdesk/rustdesk/wiki/FAQ)

Self-host your own RustDesk server, it is free and open source.

```bash
cargo build --release
```

Two executables will be generated in target/release.
  - hbbs - RustDesk ID/Rendezvous server
  - hbbr - RustDesk relay server

If you wanna develop your own server, [rustdesk-server-demo](https://github.com/rustdesk/rustdesk-server-demo) might be a better and simpler start for you than this repo.

## docker-compose

If you have Docker and would like to use it, an included `docker-compose.yml` file is included. Edit line 16 to point to your relay server (the one listening on port 21117). You can also edit the volume lines (L18 and L33) if you need.

(docker-compose credit goes to @lukebarone and @QuiGonLeong)
