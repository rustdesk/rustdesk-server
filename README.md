# RustDesk Server Program



[**Download**](https://github.com/rustdesk/rustdesk-server/releases)

[**Manual**](https://rustdesk.com/docs/en/self-host/)  

Self-host your own RustDesk server, it is free and open source.

```
cargo build --release
```

Two executables will be generated in target/release.
  - hbbs - RustDesk ID/Rendezvous server
  - hbbr - RustDesk relay server

If you wanna develop your own server, [rustdesk-server-demo](https://github.com/rustdesk/rustdesk-server-demo) might be a better and simpler start for you rather than this repo.
