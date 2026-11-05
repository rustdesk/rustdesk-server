# Modern connection types in hbbs

This document describes how `hbbs` supports the connection types that newer
RustDesk clients offer besides plain TCP hole punching and relay:

* the **server key exchange** on TCP 21116 (`KeyExchange`, versions 0 and 1),
* **IPv6 P2P** (`socket_addr_v6`),
* **WebRTC** (SDP offer/answer plus trickle ICE via `IceCandidate`),
* **UDP hole punching** (UDP `TestNatRequest`, `udp_port`, UDP `PunchHoleSent`).

The protocol is whatever the client sends and expects. Everything below was
derived from the client source at release tag **1.5.0** (hbb_common
`229b904`), and compared with tags 1.4.0 and 1.4.9. File references such as
`client.rs:863` point into `src/` of the
[client repository at 1.5.0](https://github.com/rustdesk/rustdesk/tree/1.5.0);
`webrtc.rs` is in its `libs/hbb_common/src/`. Release 1.5.1 (hbb_common
`fae47e0`) sends the same messages and expects the same replies, except that
it no longer applies a `ConfigureUpdate` from the server, alone or inside a
`TestNatResponse`
([rustdesk@9f9585c](https://github.com/rustdesk/rustdesk/commit/9f9585ce155a6558f0625eaf8fabfe8827c9a552)).
None of the new paths send one.

Every feature can be switched off; see [Switches](#switches). With all four
switches off, `hbbs` behaves like the original open-source server, except
that TCP frames on the main port are limited to 64 KiB.

---

## 1. Message flows as the client expects them

Terms: **A** is the controlling side (controller), **B** the controlled side.
B registers with `RegisterPeer`/`RegisterPk` over UDP and receives everything
`hbbs` sends to it (`PunchHole`, `FetchLocalAddr`, `RequestRelay`,
`IceCandidate`) on that UDP socket.

### 1.1 Connection attempt of the controller (1.5.0)

`Client::_start` (client.rs:407) runs up to two **rounds**, each with its own
TCP connection to hbbs:21116 and its own `PunchHoleRequest`:

* the **preferred round** carries `udp_port`, `socket_addr_v6` and
  `webrtc_sdp_offer`, if the respective option is on;
* the **fallback round** is a plain TCP punch without any of them. It only
  exists if TCP punch is allowed (`tcp_punch_allowed`, client.rs:351).

The rounds race (`race_transports_prefer_webrtc`, client.rs:220): a direct
result wins immediately, a relayed one waits `relay-fallback-delay` (2.5 s).
Each round sends its `PunchHoleRequest` up to three times (3 s, 6 s, 9 s
windows) and reads responses with `get_next_nonkeyexchange_msg`, which skips
one unsolicited `KeyExchange` (common.rs:2046).

The options `enable-udp-punch`, `enable-ipv6-punch` and `enable-webrtc` live
in the controller's LocalConfig and default to **off** for any server that is
not `*.rustdesk.com` (common.rs:1206). Explicit values win.
`enable-tcp-punch` defaults to on. The controlled side never reads these
options; it reacts to the fields that arrive.

### 1.2 Server key exchange (TCP 21116)

```
hbbs -> client  KeyExchange { keys: [sign(eph_pk)], version: 1, signed_params }
client -> hbbs  KeyExchange { keys: [client_eph_pk, sealed_key], version: picked }
                then every frame in both directions is encrypted
```

* `hbbs` sends its `KeyExchange` when a new TCP connection on 21116 has
  stayed silent for a while: 4 × the handshake round trip measured by the
  kernel plus 100 ms, at least 500 ms and at most 3 s (500 ms where the round
  trip cannot be read). A client that wants the exchange reads first, with an
  18 s timeout (common.rs:2162), so a server that stays silent costs it 18 s.
  The controlled side's plaintext replies (TCP punch `PunchHoleSent`,
  `LocalAddr`) are sent at once and the connection is closed right after,
  without reading. An unread offer would turn that close into a TCP reset,
  which can lose the reply (on packet loss, or on servers that drop unread
  data on a reset). The wait is longer than a retransmission of a lost first
  segment (RTO at least 200 ms, about three round trips after the
  handshake), so such replies get no offer. Controllers that skip the exchange may wait up to ~400 ms
  for their UDP NAT test before they send (client.rs:886); if the offer comes
  first, they skip it.
* `eph_pk` is a fresh X25519 key per connection. `keys[0]` is
  `sign(eph_pk)` with the server's Ed25519 key, the one in `id_ed25519`.
* **Version 1** (what 1.5.0 negotiates, `KX_VERSION_LATEST = 1`):
  * bit 255 of the signed `eph_pk` is set (`eph_pk[31] |= 0x80`). X25519
    ignores that bit, so older clients use the key unchanged, but a 1.5.0
    client then *requires* `signed_params` (common.rs:2177). That stops a
    downgrade by stripping the fields;
  * `signed_params = sign("rdkx-params" || KxParams { pk: eph_pk, version: 1 })`;
  * the client answers with `version = min(advertised, 1)`. For version 1
    both sides split the exchanged key into one key per direction, bound to
    the transcript (`Encrypt::new_split`, hbb_common `tcp.rs`); hbbs is the
    responder.
* **Version 0**: older clients (1.4.x) ignore `version`/`signed_params` and
  reply without `version`. hbbs then uses the single shared key.
* A client that does not want the exchange simply sends its request in
  plaintext, before or after the offer. hbbs accepts a plaintext **first**
  frame and then stays in plaintext on that connection. Only the first frame
  after the offer may be a `KeyExchange`.
* The offer needs the server's private key. If `-k` is a public key, nothing
  is offered and a warning is logged.
* Not on 21115 (NAT test port) and not on the WebSocket port 21118: WebSocket
  clients skip the exchange (common.rs:2147) and get exactly the old
  behavior.

Who uses it (1.5.0):

| Connection | Exchange |
|---|---|
| A's round carrying a WebRTC offer | `secure_tcp_required`; on failure the offer is dropped (client.rs:863) |
| A signed in (`access_token`) or switching sides | `secure_tcp`; failure is fatal (client.rs:878) |
| A's other rounds, NAT type test, B's TCP punch / LocalAddr replies | none, plaintext |
| B's `PunchHoleSent` with a WebRTC answer (TCP path), B's `RelayResponse` with an answer, B's ICE connection | `secure_tcp_required` |
| B's UDP `PunchHoleSent` with a WebRTC answer (when A sent `udp_port`) | none: the answer travels in plaintext UDP to hbbs (client design) |

The client verifies `keys[0]` with its configured `key`, falling back to
RustDesk's public key if that is empty (common.rs:2025). A client without the
right key therefore cannot complete the exchange, and gets no WebRTC.

### 1.3 TCP hole punching (unchanged)

```
A -tcp-> hbbs  PunchHoleRequest { id, nat_type, licence_key, ... }
hbbs -udp-> B  PunchHole { socket_addr = A's TCP address, nat_type, relay_server }
B -tcp-> hbbs  PunchHoleSent { socket_addr, id, relay_server, nat_type, version }   (new connection, plaintext)
hbbs -tcp-> A  PunchHoleResponse { socket_addr = B's TCP address, pk = sign(IdPk{id, pk}), ... }
```

### 1.4 UDP hole punching

```
A -udp-> hbbs:21116  TestNatRequest { serial: 0 }             (raw, unframed; repeated until answered)
hbbs -udp-> A        TestNatResponse { port = A's observed UDP port }
A -tcp-> hbbs        PunchHoleRequest { ..., udp_port }
hbbs -udp-> B        PunchHole { ..., udp_port }
B -udp-> hbbs        PunchHoleSent { socket_addr = A's TCP address, id, ... }   (3 copies, from a fresh UDP socket)
hbbs -tcp-> A        PunchHoleResponse { socket_addr = B's observed UDP address, is_udp: true, ... }
A <-udp-> B          "RDP?"/"RDP!" probes, then KCP
```

* `test_udp_uat` (client.rs:5448) sends the request from the UDP socket it
  later punches with, so `udp_port` is that socket's public port. 1.5.0 takes
  `udp_port` only from hbbs; the original hbbs never answers UDP
  `TestNatRequest`, so a 1.5.0 controller always sends `udp_port = 0` to it.
* B punches A at `PunchHole.socket_addr`'s IP with port `udp_port`.
* B sends `PunchHoleSent` over UDP from the socket it then uses for KCP
  (rendezvous_mediator.rs:1209). It sends no TCP reply on this path, and
  expects nothing back over UDP. hbbs must answer A **over A's TCP
  connection**, with the observed source address of that UDP datagram.
* A uses UDP only if it sent `udp_port > 0` and gets `is_udp = true`
  (client.rs:1021).
* hbbs forwards `udp_port` only from controllers 1.5.0 or newer. 1.4.x
  controllers send it too, but their UDP attempt (`punch_udp`, 20 s limit,
  no outer timeout) holds up the relay fallback when UDP fails, and 1.4.1 has
  no parallel TCP round that could win meanwhile; the controlled side does no
  TCP punch once it gets a `udp_port`. With the original server, which
  dropped `udp_port`, those clients always got a TCP punch.

### 1.5 IPv6 P2P

* A puts its own public IPv6 address and the port of its v6 UDP socket into
  `PunchHoleRequest.socket_addr_v6` (self-reported, client.rs:508). Only
  global addresses (2000::/3) or a STUN result are used (common.rs:2633).
* hbbs copies it into `PunchHole.socket_addr_v6`, or `FetchLocalAddr.socket_addr_v6`
  in the same-intranet case, under these conditions:
  * the controller is 1.5.0 or newer (`PunchHoleRequest.version`). 1.4.x
    controllers also send the field, but their IPv6 attempt
    (`udp_nat_connect`, `punch_udp` with a 20 s limit) holds up the relay
    fallback for up to 20 s when IPv6 does not work, which the original
    server never caused;
  * the controller did not ask for a relay itself (`force_relay`; 1.4.x sends
    its address even then);
  * the address is global unicast (2000::/3) with a port, which is all a
    genuine client reports;
  * the controller's IP has not handed out more than 60 addresses in a burst
    (1 per second after that), and the same target /64 has not been handed
    out more than that by all controllers together, because the peer sends
    punch probes to whatever address it gets.
* B opens a v6 UDP listener and reports it in `PunchHoleSent.socket_addr_v6`,
  `LocalAddr.socket_addr_v6` or `RelayResponse.socket_addr_v6`. B does this
  only when it got A's address (1.4.9 and 1.5.0 alike).
* hbbs copies that into `PunchHoleResponse.socket_addr_v6` or
  `RelayResponse.socket_addr_v6` only on the way back to a controller whose
  own address went to B (so never past a forced relay or to a 1.4.x
  controller), only from B's registered host, again only global unicast with
  a port, and within the same per-target limit. A starts an "IPv6" KCP
  attempt when the port is non-zero (client.rs:1028, 1049).

### 1.6 WebRTC

```
A -tcp(kx)-> hbbs  PunchHoleRequest { ..., webrtc_sdp_offer, force_relay }
hbbs -udp-> B      PunchHole { ..., webrtc_sdp_offer, force_relay }
B -> hbbs          answer in one of:
                     PunchHoleSent.webrtc_sdp_answer over UDP (if udp_port > 0)
                     PunchHoleSent.webrtc_sdp_answer over a new TCP connection with key exchange
                     RelayResponse.webrtc_sdp_answer over a new TCP connection with key exchange (relay case)
hbbs -tcp-> A      PunchHoleResponse/RelayResponse { ..., webrtc_sdp_answer }
trickle ICE:
A -tcp-> hbbs      IceCandidate { id = B's id, session_key, candidate }   (A's punch connection, stays open)
hbbs -udp-> B      IceCandidate { session_key, candidate }
B -tcp(kx)-> hbbs  IceCandidate { socket_addr = A's address from PunchHole, session_key, candidate }
                   (one dedicated connection for all of B's candidates)
hbbs -tcp-> A      IceCandidate { session_key, candidate }                (on A's punch connection)
```

* The offer and answer are `webrtc://` + base64(JSON `{type, sdp,
  ice_policy}`), about 700 bytes, without candidates (webrtc.rs:993).
* `session_key` is the DTLS fingerprint of the **offer** SDP (`a=fingerprint`),
  e.g. `sha-256 AB:CD:…`. Both peers match candidates only by it
  (client.rs:590, rendezvous_mediator.rs:560).
* A sends candidates only after it has the answer, and each one twice
  (400 ms apart). It keeps the punch connection open while `connect()` runs
  and never reconnects it (client.rs:671).
* A round carrying an offer never does a TCP punch (client.rs:340); TCP is
  the fallback round's job.
* The controlled side sends its answer over TCP only after the key exchange
  (rendezvous_mediator.rs:1172) and silently gives up if it fails, so without
  the server's key in its `key` option it answers only on the UDP path. When
  A sent a `udp_port`, B puts the answer into its UDP `PunchHoleSent`
  instead (rendezvous_mediator.rs:1146, `punch_udp_hole`), without any key
  exchange; hbbs forwards it from B's registered host. So with UDP punching,
  WebRTC can start without the key on B, and that answer (ICE credentials,
  DTLS fingerprint) crosses the network to hbbs unencrypted. That is the
  client's design; the DTLS handshake and RustDesk's own check of the peer's
  signed key still protect the session. B's own candidates go over a separate
  TCP connection that it uses only after a key exchange
  (rendezvous_mediator.rs:961), so without the key B sends none, and the
  connection has to come up from A's candidates alone. hbbs forwards the offer only twice per controller
  connection while nothing comes back; a third (re-sent) request goes without
  it, the peer answers with a plain punch and the controller can still
  relay, as with the original server.
* Same intranet: `FetchLocalAddr` has no offer field. A request carrying an
  offer therefore gets a `PunchHole` (WebRTC uses LAN host candidates). The
  offer-less fallback round still gets `FetchLocalAddr` for the direct LAN
  TCP connection.

### 1.7 Relay

Unchanged: `RequestRelay` from A, `RelayResponse` from B. The relay leg to
hbbr is not affected; hbbr is not changed.

---

## 2. Server design

### Switches

| Env var | CLI flag | Default | Off means |
|---|---|---|---|
| `SECURE_TCP` | `--secure-tcp` | `Y` | no `KeyExchange` is offered (original behavior). WebRTC then cannot work, because the clients require the exchange for it. |
| `IPV6_PUNCH` | `--ipv6-punch` | `Y` | `socket_addr_v6` is dropped from every forwarded message |
| `WEBRTC` | `--webrtc` | `Y` | offers and answers are dropped; `IceCandidate` closes the connection as before |
| `UDP_PUNCH` | `--udp-punch` | `Y` | UDP `TestNatRequest` is not answered, `udp_port` is dropped, UDP `PunchHoleSent` is ignored |

Values: `N`, `no`, `0`, `false` or `off` (case-insensitive) turn a feature
off; `Y`, `yes`, `1`, `true`, `on` or nothing leave it on; any other value
leaves it on and logs a warning.

### Punch routes

A **punch route** is the server's record of one controller connection that
asked for UDP and/or WebRTC or got its IPv6 address forwarded. It is keyed by
A's TCP address (the `tcp_punch` key) and holds:

* the target peer id;
* B's registered IP at the time of the request (for the sender checks);
* whether UDP was granted, and whether the UDP answer was already used;
* whether A's IPv6 address went to B;
* for WebRTC, the session key taken from the offer, how often the offer was
  forwarded, and the number of candidates forwarded in each direction;
* whether any response from B reached A;
* creation and expiry time.

A route is created only by a `PunchHoleRequest` that passed every existing
check: license key, peer exists, peer online. It is removed when A's
connection closes or when it expires.

While a WebRTC route is alive, A's TCP sink stays in `tcp_punch` after the
first response, so that the ICE candidates can reach A. It still serves only
one response: further replies for A's address (which anyone can forge) are
dropped, so they cannot queue up behind a controller that stopped reading.
Without a WebRTC route, the sink is consumed by the first response, as
before.

### Handlers

| Message | Arrives | New behavior |
|---|---|---|
| (accept) | TCP 21116 | send `KeyExchange` v1 if the client stays silent for 500 ms or more (`SECURE_TCP`) |
| `KeyExchange` | TCP, first frame | complete the exchange (v0 or v1); afterwards decrypt/encrypt every frame |
| `TestNatRequest` | UDP | `TestNatResponse { port }` without `ConfigUpdate`, rate-limited (`UDP_PUNCH`) |
| `PunchHoleRequest` | TCP | forward `udp_port`, `socket_addr_v6`, `webrtc_sdp_offer` (+`force_relay` with an offer) in `PunchHole`; v6 also in `FetchLocalAddr`; open a punch route. No `udp_port` or v6 when the controller set `force_relay`; `udp_port` and v6 only from 1.5.0 controllers |
| `PunchHoleSent` | TCP | forward `socket_addr_v6` and `webrtc_sdp_answer` if A's route got them (see below) |
| `PunchHoleSent` | UDP | if it matches an open UDP route and comes from B's registered host: `PunchHoleResponse { socket_addr = observed UDP source, is_udp: true }` to A over TCP; otherwise ignored |
| `LocalAddr` | TCP | forward `socket_addr_v6` if A's route got it |
| `RelayResponse` | TCP | forward `socket_addr_v6` and `webrtc_sdp_answer` if A's route got them |
| `IceCandidate` from A | TCP (A's punch connection) | forward to B's registered UDP address if it matches A's route |
| `IceCandidate` from B | TCP (B's ICE connection) | forward to A's punch connection if it matches the route of `socket_addr`; the connection stays open |

A reply carries a WebRTC answer or an IPv6 address back only if A's route
granted it, the reply names the route's peer id, and it comes from B's
registered host (IPv4: equal; IPv6: same /64), never over WebSocket. Anyone
who knows A's address can send a reply, as with the original server; such a
reply still reaches A as before, but without these fields.

WebSocket connections (21118) keep the original behavior: no key exchange, no
new fields, no `IceCandidate` handling.

`ALWAYS_USE_RELAY=Y`, and the LAN/WAN mismatch that forces relay, also drop
`udp_port`, `socket_addr_v6` and the offer, because IPv6 and WebRTC would
otherwise bypass the forced relay.

---

## 3. Security design

### 3.1 UDP: one small, rate-limited reply (CVE-2026-30784, issue #670)

The original server answered several UDP messages with traffic to addresses
taken from the packet, which made it a reflector. 1.1.16 removed those paths.

The new UDP handling adds exactly one reply to an address taken from a
packet: the answer to the UDP NAT test. UDP hole punching cannot do without
it, because the client learns its public UDP port from it, as it would from
a STUN server. Like a STUN answer, it goes to the packet's source address, so
a spoofed source makes hbbs send it to the spoofed address. That reflection
is bounded as described below: one packet per packet, at most ~1.15x the
bytes, and at most 30 answers per second (burst 60) to one address (IPv6:
one /64), which is about 1 KB/s. Nothing else in the new UDP handling sends
to an address taken from a packet, and `UDP_PUNCH=N` removes the reply.

* **UDP `TestNatRequest`** is the only new UDP message that gets a UDP reply.
  * The reply goes only to the packet's source address. It is
    `TestNatResponse { port }` only, never a `ConfigUpdate`: at most 7 bytes of
    payload for the client's 3-byte request. On the wire (with IP/UDP headers)
    that is at most ~1.15x, one packet per packet.
  * It is rate-limited per source IP (IPv6: per /64) with a token bucket
    (burst 60, 30 per second: a client's test sends 5-7 requests, and
    several clients can share one public IP), and globally
    (`UDP_TEST_NAT_LIMIT`, default 2000 answers per second). The limiter
    table holds at most 65,536 sources; when it is full, unknown sources are
    dropped (fail closed), or held to the global budget if they are proven
    (below).
  * Spoofed sources could use up the global budget and so switch off UDP
    punching for everyone. Sources with an open TCP connection to hbbs are
    therefore exempt from the global budget (not from their own): the TCP
    handshake proves their address. A new unproven source only gets an entry
    in the limiter table while the global budget has tokens left, so spoofed
    sources cannot fill the table either.
  * Residual risk: a controller starts its UDP NAT test just before it opens
    its rendezvous connection and then waits about half a round trip
    (client.rs:891). During a flood that keeps the global budget empty, its
    early requests are refused, and with a round trip above ~100 ms the
    proven retries come too late. UDP punching then falls back to TCP or
    relay, which is what the original server always did.
  * No reply goes to port 0, or to unspecified, multicast or broadcast
    addresses. The reply is sent without waiting, and a send error is
    logged and only drops it. Returned as an error, it would make hbbs
    rebuild its UDP socket, which a refused (spoofed) destination could then
    trigger at will. An oversized `RequestRelay` forwarded to a peer (TCP
    frames go up to 64 KiB, a datagram only to 65,507 bytes) is dropped the
    same way: hbb_common keeps no failed datagram queued (since `fae47e0`),
    so later sends are not affected.
* **UDP `PunchHoleSent`** never causes a UDP packet. It is accepted only if
  * an open punch route exists for the decoded `socket_addr` (A's address),
    created by a verified TCP `PunchHoleRequest` that asked for UDP;
  * the route's target id equals `PunchHoleSent.id`;
  * the route is not expired and its UDP answer has not been used (B sends 3
    copies; only the first counts);
  * the datagram's source IP matches B's registered IP (IPv4: equal; IPv6:
    same /64).

  Then hbbs sends one `PunchHoleResponse` with `is_udp` to A **over A's
  existing TCP connection**. Anything else is dropped silently and leaves the
  route as it was, so a forged datagram cannot use up the route that B's
  reply needs. A UDP source proves nothing, so a datagram from another host
  (forged, or B behind a NAT pool or multi-WAN router) counts as lost: A's
  parallel TCP round or its retries go on, as after any lost datagram.
* UDP `PunchHoleRequest`, `LocalAddr` and `IceCandidate` stay ignored.
* Messages hbbs sends to B over UDP (`PunchHole`, `IceCandidate`) go only to
  B's registered address. They are triggered by A's TCP connection, which
  cannot be spoofed, has passed the license key check, and is limited by the
  route's budgets.

### 3.1a Addresses the peers punch

The peers themselves send probes to addresses that travel through hbbs. That
is how hole punching works, and the same in RustDesk Server Pro, but it can
be abused to make a peer send packets to a third host:

* `udp_port`: the peer punches the controller's **observed** IP with that
  port, so it cannot be pointed at another host.
* `socket_addr_v6`: chosen by the controller. The controlled peer sends its
  punch probes there: for up to 3 s (1.5.0, `punch_udp`) or 20 s (1.4.x,
  about 100 datagrams). hbbs forwards only global unicast addresses with a
  port, at most 60 per controller IP and 60 per target /64 in a burst, then
  1 per second each; a request also needs the license key and an online
  peer id. A target in the controller's own /64 (a controller that reaches
  hbbs over IPv6) counts only per controller: the probes go to its own
  network, and nobody else can use up its budget by naming it. For a
  controller on IPv4 this cannot be proven, so others can still switch its
  IPv6 P2P off by naming its /64 (it falls back to the other transports).
  The "1.5.0 or newer" check is about compatibility, not security: the
  version is reported by the client itself.
* ICE candidates: chosen by each side; the peer's ICE agent sends paced
  connectivity checks to them, as in any WebRTC implementation. Bounded by
  128 candidates per route and 64 routes per controller IP.
* B's `socket_addr_v6` on the way back: A punches it. hbbs forwards it only
  to a controller whose own address went to B, only from B's registered host,
  and within the same per-target limit, so neither a third party that knows
  A's address nor B itself can point many controllers at one host.

### 3.2 Bounded state

| State | Bound |
|---|---|
| Punch routes | 10,000 total, 64 per controller IP (IPv6: per /64), TTL 30 s (UDP or IPv6 only) / 60 s (WebRTC), refreshed by re-sent offers but never beyond 120 s after creation; after that a new request opens a new route. When the table is full, a newcomer takes the oldest route of the source with clearly the most routes, so one heavy user cannot lock out the others |
| ICE per route | 128 candidates per direction (the controller sends each candidate twice, so 64 distinct ones); more are dropped quietly |
| ICE frames per connection | 256; a connection with more than 16 candidate frames that are invalid (empty, too long, no session key) or cannot be forwarded (no matching route, session or connection) is closed |
| SDP offer/answer | 4096 bytes, otherwise dropped |
| ICE candidate | 2048 bytes; session key 256 bytes |
| `socket_addr_v6` | 18 bytes (the mangled IPv6 form), otherwise dropped |
| UDP `TestNatRequest` limiter | per IP/64: burst 60, 30/s; global: 2000/s (`UDP_TEST_NAT_LIMIT`), burst twice that, not applied to sources with an open TCP connection; table at most 65,536 entries, a full table is swept at most every 2 s |
| IPv6 forwards | per controller IP/64 and per target /64: burst 60, 1/s each (a target in the sender's own /64 counts only per sender); tables at most 65,536 entries; a full controller table lets unknown controllers pass (they are licensed and cannot be spoofed), a full target table refuses unknown targets |
| TCP frames on 21116 | 64 KiB per frame |
| Key exchange | per-connection only |

`tcp_punch` entries already live only as long as their TCP connection.

### 3.3 Other rules

* **License key**: routes are only created after the existing key check in
  `PunchHoleRequest`. Every new forwarding path (answers, candidates, UDP
  `PunchHoleSent`) needs such a route, so the key check covers it. UDP
  `TestNatRequest` carries no key, like its existing TCP counterpart; it only
  reveals the sender's own public port.
* **Logging**: keys, SDPs and ICE candidates are never logged, only sizes and
  counts.
* **Untrusted input**: no `unwrap()` on external data; mangled addresses from
  the network are decoded with `punch::decode_addr`, a checked version of
  `AddrMangle::decode`, which subtracts without a check and so panics on
  crafted input in debug builds; malformed packets are dropped.
* **ICE injection**: a candidate is only forwarded if its `session_key`
  matches the DTLS fingerprint in the route's offer. A third party that knows
  A's address cannot inject candidates without it. The DTLS fingerprint
  binding in the clients (`dtls_fingerprint_bound`, common.rs:2296) still protects against a
  man-in-the-middle.
* **Sink lifetime**: a WebRTC route keeps A's sink at most until the route
  expires or A disconnects, for one response and the route's candidates.
  Every write to a kept sink gives up after 3 s, waiting for it included.

---

## 4. Compatibility

### Clients

| Client | Effect |
|---|---|
| 1.5.0 | all four features work when the controller enables them |
| 1.4.1 – 1.4.9 | as controller: no WebRTC, and its `udp_port` and `socket_addr_v6` are not forwarded (see 1.4 and 1.5), so it connects exactly as with the original server; the exchange runs as v0. As controlled side: once it has replied with its version (a `PunchHoleSent`, `LocalAddr` or `RelayResponse` to a waiting controller, sent from its registered host over plain TCP or a claimed UDP route; never over WebSocket, whose address can come from a header), hbbs remembers it in memory and sends it no `udp_port` and no IPv6 address, so it does its TCP punch as with the original server. Only before its first reply after a server start can it get them, or always if its replies leave from another IP than its registration (multi-WAN, NAT pool without paired addresses); it then punches UDP with probes 1.5.0 does not answer (`RDP?`/`RDP!` came with 1.5.0), that round ends on the relay after ~3 s, and the controller's parallel TCP round usually wins first. WebRTC offers are ignored by it; the controller falls back. |
| 1.4.0 | knows none of the new fields; they are not sent to it in a way it can misread. The exchange runs as v0 when it is signed in. |
| older | not tested; the unsolicited `KeyExchange` is skipped by every client that reads responses with `get_next_nonkeyexchange_msg` |
| all options off | as before: plain `PunchHoleRequest`, TCP punch or relay (the server's 64 KiB frame limit applies to every client) |

Behavior changes that are improvements, not regressions:

* Signed-in 1.4.9/1.5.0 clients failed against the original hbbs after 18 s
  ("Failed to secure tcp: deadline has elapsed", rustdesk-server#394). With
  `SECURE_TCP` they work if their `key` matches the server key. With a wrong
  key they still fail, now immediately ("Signature mismatch").
* Signed-in 1.4.0 clients lost 18 s; now they get an encrypted connection.
* Clients call the API of RustDesk Server Pro in the background and, when
  plain HTTP fails, retry through hbbs (`HttpProxyRequest` after the key
  exchange, common.rs:1528). The open-source hbbs has no API either way;
  such a retry used to wait 18 s for the key exchange and now fails at once,
  because the message is unknown and the connection is closed as before.
* Clients that register over TCP (proxy, WebSocket, `disable-udp`) could not
  register before and still cannot: `RegisterPk` over TCP is answered with
  `NOT_SUPPORT` as before. With `SECURE_TCP` they now get that answer after a
  successful exchange instead of an 18 s timeout.

### Server state

The database schema and the key files are unchanged. Switching back to the
original image keeps working.

### WebSocket

The WebSocket port gets no key exchange and none of the new fields, as before.
