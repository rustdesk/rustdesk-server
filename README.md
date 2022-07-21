# RustDesk Server Program

[![build](https://github.com/rustdesk/rustdesk-server/actions/workflows/build.yaml/badge.svg)](https://github.com/rustdesk/rustdesk-server/actions/workflows/build.yaml)

[**Download**](https://github.com/rustdesk/rustdesk-server/releases)

[**Manual**](https://rustdesk.com/docs/en/self-host/)

[**FAQ**](https://github.com/rustdesk/rustdesk/wiki/FAQ)

Self-host your own RustDesk server, it is free and open source.

## How to build manually

```bash
cargo build --release
```

Two executables will be generated in target/release.

- hbbs - RustDesk ID/Rendezvous server
- hbbr - RustDesk relay server

You can find updated binaries on the [releases](https://github.com/rustdesk/rustdesk-server/releases) page.

If you wanna develop your own server, [rustdesk-server-demo](https://github.com/rustdesk/rustdesk-server-demo) might be a better and simpler start for you than this repo.

## Docker images

Docker images are automatically generated and published on every github release. We have 2 kind of images.

### Classic image

These images are build against `ubuntu-20.04` with the only addition of the binaries (both hbbr and hbbs). They're available on [Docker hub](https://hub.docker.com/r/rustdesk/rustdesk-server/) with these tags:

| architecture | image:tag |
| --- | --- |
| amd64 | `rustdesk/rustdesk-server:latest` |
| arm64v8 | `rustdesk/rustdesk-server:latest-arm64v8` |

You can start these images directly with `docker run` with these commands:

```bash
docker run --name hbbs --net=host -v "$PWD:/root" -d rustdesk/rustdesk-server:latest hbbs -r <relay-server-ip[:port]> 
docker run --name hbbr --net=host -v "$PWD:/root" -d rustdesk/rustdesk-server:latest hbbr 
```

or without --net=host, but P2P direct connection can not work.

```bash
docker run --name hbbs -p 21115:21115 -p 21116:21116 -p 21116:21116/udp -p 21118:21118 -v "$PWD:/root" -d rustdesk/rustdesk-server:latest hbbs -r <relay-server-ip[:port]> 
docker run --name hbbr -p 21117:21117 -p 21119:21119 -v "$PWD:/root" -d rustdesk/rustdesk-server:latest hbbr 
```

The `relay-server-ip` parameter is the IP address (or dns name) of the server running these containers. The **optional** `port` parameter has to be used if you use a port different than **21117** for `hbbr`.

You can also use docker-compose, using this configuration as a template:

```yaml
version: '3'

networks:
  rustdesk-net:
    external: false

services:
  hbbs:
    container_name: hbbs
    ports:
      - 21115:21115
      - 21116:21116
      - 21116:21116/udp
      - 21118:21118
    image: rustdesk/rustdesk-server:latest
    command: hbbs -r rustdesk.example.com:21117
    volumes:
      - ./hbbs:/root
    networks:
      - rustdesk-net
    depends_on:
      - hbbr
    restart: unless-stopped

  hbbr:
    container_name: hbbr
    ports:
      - 21117:21117
      - 21119:21119
    image: rustdesk/rustdesk-server:latest
    command: hbbr
    volumes:
      - ./hbbr:/root
    networks:
      - rustdesk-net
    restart: unless-stopped
```

Edit line 16 to point to your relay server (the one listening on port 21117). You can also edit the volume lines (L18 and L33) if you need.

(docker-compose credit goes to @lukebarone and @QuiGonLeong)

## S6-overlay based images

These images are build against `busybox:stable` with the addition of the binaries (both hbbr and hbbs) and [S6-overlay](https://github.com/just-containers/s6-overlay). They're available on [Docker hub](https://hub.docker.com/r/rustdesk/rustdesk-server-s6/) with these tags:

| architecture | version | image:tag |
| --- | --- | --- |
| multiarch | latest | `rustdesk/rustdesk-server-s6:latest` |
| amd64 | latest | `rustdesk/rustdesk-server-s6:latest-amd64` |
| i386 | latest | `rustdesk/rustdesk-server-s6:latest-i386` |
| arm64v8 | latest | `rustdesk/rustdesk-server-s6:latest-arm64v8` |
| armv7 | latest | `rustdesk/rustdesk-server-s6:latest-armv7` |
| multiarch | 2 | `rustdesk/rustdesk-server-s6:2` |
| amd64 | 2 | `rustdesk/rustdesk-server-s6:2-amd64` |
| i386 | 2 | `rustdesk/rustdesk-server-s6:2-i386` |
| arm64v8 | 2 | `rustdesk/rustdesk-server-s6:2-arm64v8` |
| armv7 | 2 | `rustdesk/rustdesk-server-s6:2-armv7` |
| multiarch | 2.0.0 | `rustdesk/rustdesk-server-s6:2.0.0` |
| amd64 | 2.0.0 | `rustdesk/rustdesk-server-s6:2.0.0-amd64` |
| i386 | 2.0.0 | `rustdesk/rustdesk-server-s6:2.0.0-i386` |
| arm64v8 | 2.0.0 | `rustdesk/rustdesk-server-s6:2.0.0-arm64v8` |
| armv7 | 2.0.0 | `rustdesk/rustdesk-server-s6:2.0.0-armv7` |

You're strongly encuraged to use the `multiarch` image either with the `major version` or `latest` tag.

The S6-overlay acts as a supervisor and keeps both process running, so with this image there's no need to have two separate running containers.

You can start these images directly with `docker run` with this command:

```bash
docker run --name rustdesk-server \ 
  --net=host \
  -e "RELAY=rustdeskrelay.example.com" \
  -e "ENCRYPTED_ONLY=1" \
  -v "$PWD/data:/data" -d rustdesk/rustdesk-server-s6:latest
```

or without --net=host, but P2P direct connection can not work.

```bash
docker run --name rustdesk-server \
  -p 21115:21115 -p 21116:21116 -p 21116:21116/udp \
  -p 21117:21117 -p 21118:21118 -p 21119:21119 \
  -e "RELAY=rustdeskrelay.example.com" \
  -e "ENCRYPTED_ONLY=1" \
  -v "$PWD/data:/data" -d rustdesk/rustdesk-server-s6:latest
```

Or you can use a docker-compose file:

```yaml
version: '3'

services:
  rustdesk-server:
    container_name: rustdesk-server
    ports:
      - 21115:21115
      - 21116:21116
      - 21116:21116/udp
      - 21117:21117
      - 21118:21118
      - 21119:21119
    image: rustdesk/rustdesk-server-s6:latest
    environment:
      - "RELAY=rustdesk.example.com:21117"
      - "ENCRYPTED_ONLY=1"
    volumes:
      - ./data:/data
    restart: unless-stopped
```

We use these environment variables:

| variable | optional | description |
| --- | --- | --- |
| RELAY | no | the IP address/DNS name of the machine running this container |
| ENCRYPTED_ONLY | yes | if set to **"1"** unencrypted connection will not be accepted |
| DB_URL | yes | path for database file |
| KEY_PUB | yes | public part of the key pair |
| KEY_PRIV | yes | private part of the key pair |

### Secret management in S6-overlay based images

You can obviously keep the key pair in a docker volume, but the best practices tells you to not write the keys on the filesystem; so we provide a couple of options.

On container startup, the presence of the keypair is checked (`/data/id_ed25519.pub` and `/data/id_ed25519`) and if one of these keys doesn't exist, it's recreated from ENV variables or docker secrets.

#### Use ENV to store the key pair

You can use docker environment variables to store the keys. Just follow this examples:

```bash
docker run --name rustdesk-server \ 
  --net=host \
  -e "RELAY=rustdeskrelay.example.com" \
  -e "ENCRYPTED_ONLY=1" \
  -e "DB_URL=/db/db_v2.sqlite3" \
  -e "KEY_PRIV=FR2j78IxfwJNR+HjLluQ2Nh7eEryEeIZCwiQDPVe+PaITKyShphHAsPLn7So0OqRs92nGvSRdFJnE2MSyrKTIQ==" \
  -e "KEY_PUB=iEyskoaYRwLDy5+0qNDqkbPdpxr0kXRSZxNjEsqykyE=" \
  -v "$PWD/db:/db" -d rustdesk/rustdesk-server-s6:latest
```

```yaml
version: '3'

services:
  rustdesk-server:
    container_name: rustdesk-server
    ports:
      - 21115:21115
      - 21116:21116
      - 21116:21116/udp
      - 21117:21117
      - 21118:21118
      - 21119:21119
    image: rustdesk/rustdesk-server-s6:latest
    environment:
      - "RELAY=rustdesk.example.com:21117"
      - "ENCRYPTED_ONLY=1"
      - "DB_URL=/db/db_v2.sqlite3"
      - "KEY_PRIV=FR2j78IxfwJNR+HjLluQ2Nh7eEryEeIZCwiQDPVe+PaITKyShphHAsPLn7So0OqRs92nGvSRdFJnE2MSyrKTIQ=="
      - "KEY_PUB=iEyskoaYRwLDy5+0qNDqkbPdpxr0kXRSZxNjEsqykyE="
    volumes:
      - ./db:/db
    restart: unless-stopped
```

#### Use Docker secrets to store the key pair

You can alternatively use docker secrets to store the keys.
This is useful if you're using **docker-compose** or **docker swarm**.
Just follow this examples:

```bash
cat secrets/id_ed25519.pub | docker secret create key_pub -
cat secrets/id_ed25519 | docker secret create key_priv -
docker service create --name rustdesk-server \
  --secret key_priv --secret key_pub \
  --net=host \
  -e "RELAY=rustdeskrelay.example.com" \
  -e "ENCRYPTED_ONLY=1" \
  -e "DB_URL=/db/db_v2.sqlite3" \
  --mount "type=bind,source=$PWD/db,destination=/db" \
  rustdesk/rustdesk-server-s6:latest
```

```yaml
version: '3'

services:
  rustdesk-server:
    container_name: rustdesk-server
    ports:
      - 21115:21115
      - 21116:21116
      - 21116:21116/udp
      - 21117:21117
      - 21118:21118
      - 21119:21119
    image: rustdesk/rustdesk-server-s6:latest
    environment:
      - "RELAY=rustdesk.example.com:21117"
      - "ENCRYPTED_ONLY=1"
      - "DB_URL=/db/db_v2.sqlite3"
    volumes:
      - ./db:/db
    restart: unless-stopped
    secrets:
      - key_pub
      - key_priv

secrets:
  key_pub:
    file: secrets/id_ed25519.pub
  key_priv:
    file: secrets/id_ed25519      
```

## .deb packages

.deb packages are available for each binary, you can find them in the [releases](https://github.com/rustdesk/rustdesk-server/releases).
These packages are meant for the following distributions:

- Ubuntu 22.04 LTS
- Ubuntu 20.04 LTS
- Ubuntu 18.04 LTS

Debian is currently not supported.
