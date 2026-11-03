# Network resource limits

Both servers cap active network connections and reject oversized messages before
buffering their bodies. Set these environment variables in the server's existing
configuration and restart the process to change the limits:

| Variable | Default | Accepted range |
| --- | --- | --- |
| `MAX_CONNECTIONS` | 1024 per process | 1–65536 |
| `MAX_CONTROL_MESSAGE_SIZE` | 1048576 bytes (1 MiB) | 1–67108864 |
| `MAX_WEBSOCKET_MESSAGE_SIZE` | 16777216 bytes (16 MiB) | 1–67108864 |

Invalid, zero, and out-of-range values use the defaults. Choose limits appropriate
for available memory and expected concurrency; the maximum accepted settings are
not recommendations. These are resource controls, not licensed-device limits.

`MAX_CONNECTIONS` covers signaling, NAT/online queries, relay waiters, HTTP upgrade
handshakes, and paired relay sockets. Each socket retains its permit until it is
dropped, including the first socket transferred into a relay pair. New connections
are closed when capacity is exhausted; existing sessions continue. The existing
loopback-only administrative console is excluded so administrators retain access.

`MAX_CONTROL_MESSAGE_SIZE` covers framed TCP control traffic, the initial TCP
relay request, and signaling WebSocket frames/messages. It permits large batches
of online-status queries; increase it if a deployment needs larger batches.
Paired raw TCP relay traffic continues streaming without this control-frame cap.

`MAX_WEBSOCKET_MESSAGE_SIZE` caps relay WebSocket frames and complete messages,
including fragmented messages, throughout the connection. This is deliberately
larger than the control limit to accommodate binary relay traffic. Deployments
requiring larger individual WebSocket messages can increase it up to 64 MiB,
the previous message-size default. WebSocket ports should still be reachable
only through the intended reverse proxy.
