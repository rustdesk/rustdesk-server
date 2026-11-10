# Peer and IP cache limits

`hbbs` cleans inactive bookkeeping every 60 seconds. Peer entries expire after
10 minutes without a registration or registration attempt. Online peers remain
cached, and entries still held by an in-flight operation are retained until a
later cleanup. Durable device records in SQLite are never removed by this task.
IP throttling counters retain their existing 60-second/day windows; IP-change
diagnostics expire after their existing six-minute window.

Set these environment variables before starting `hbbs` to size the caches:

| Variable | Default | Accepted range |
| --- | --- | --- |
| `MAX_PEER_CACHE_ENTRIES` | 100000 | 1–1000000 |
| `MAX_IP_CACHE_ENTRIES` | 10000 | 1–1000000 |

Invalid values use the defaults. These are memory limits, not licensing limits.
Existing cached peers/IPs remain usable at capacity; new registrations receive
`TOO_FREQUENT` until cleanup frees capacity or an administrator raises the limit.
Durable database lookups remain readable even when the cache is full. Bringing
another uncached device online still needs a cache slot and can receive the
capacity response above; the configured bound cannot admit unlimited active
devices. No live peers are evicted to make room. New IP-change diagnostics
are skipped at capacity, and each peer's diagnostic list holds at most 128 IPs.

Registration public keys must be 32-byte Ed25519 keys, as used by RustDesk.
Device UUIDs must be nonempty and at most 1024 bytes; peer IDs retain the existing
253-byte bound. Malformed registration fields are rejected before cache/storage.
