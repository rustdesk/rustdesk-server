<p align="center">
  <a href="#hoe-handmatig-opbouwen">Opbouwen</a> •
  <a href="#docker-bestanden-images">Docker</a> •
  <a href="#s6-overlay-gebaseerde-bestanden">S6-Overlay</a> •
  <a href="#hoe-maak-je-een-key-paar">Key paar</a> •
  <a href="#deb-pakketten">Debian pakketten</a> •
  <a href="#env-variabelen">ENV variabelen</a><br>
  [<a href="README.md">English</a>] | [<a href="README-DE.md">Deutsch</a>] | [<a href="README-TW.md">繁體中文</a>] | [<a href="README-ZH.md">简体中文</a>]<br>
</p>

# RustDesk Server Programa

[![build](https://github.com/rustdesk/rustdesk-server/actions/workflows/build.yaml/badge.svg)](https://github.com/rustdesk/rustdesk-server/actions/workflows/build.yaml)

[**Download**](https://github.com/rustdesk/rustdesk-server/releases)

[**Handleiding**](https://rustdesk.com/docs/nl/self-host/)

[**FAQ**](https://github.com/rustdesk/rustdesk/wiki/FAQ)

Zelf uw eigen RustDesk server hosten, het is gratis en open source.

## Hoe handmatig opbouwen

```bash
cargo build --release
```

In target/release worden drie uitvoerbare bestanden gegenereerd.

- hbbs - RustDesk ID/Rendezvous server
- hbbr - RustDesk relay server
- rustdesk-utils - RustDesk CLI hulpprogramma's

U kunt bijgewerkte binaries vinden op [releases](https://github.com/rustdesk/rustdesk-server/releases) pagina.

Als u uw eigen server wilt ontwikkelen, is [rustdesk-server-demo](https://github.com/rustdesk/rustdesk-server-demo) misschien een betere en eenvoudigere start voor u dan deze repo.

## Docker bestanden (images)

Docker bestanden (images) worden automatisch gegenereerd en gepubliceerd bij elke github release. We hebben 2 soorten bestanden (images).

### Klassiek bestand (image)

Deze bestanden (images) zijn gebouwd voor `ubuntu-20.04` met als enige toevoeging de belangrijkste binaries (`hbbr` en `hbbs`). Ze zijn beschikbaar op [Docker hub](https://hub.docker.com/r/rustdesk/rustdesk-server/) met deze tags:

| architectuur | image:tag |
| --- | --- |
| amd64 | `rustdesk/rustdesk-server:latest` |
| arm64v8 | `rustdesk/rustdesk-server:latest-arm64v8` |

U kunt deze bestanden (images) direct starten via `docker run` met deze commando's:

```bash
docker run --name hbbs --net=host -v "$PWD/data:/root" -d rustdesk/rustdesk-server:latest hbbs -r <relay-server-ip[:port]> 
docker run --name hbbr --net=host -v "$PWD/data:/root" -d rustdesk/rustdesk-server:latest hbbr 
```

of zonder `--net=host`, maar een directe P2P verbinding zal niet werken.

Voor systemen die SELinux gebruiken is het vervangen van `/root` door `/root:z` nodig om de containers correct te laten draaien. Als alternatief kan SELinux containerscheiding volledig worden uitgeschakeld door de optie `--security-opt label=disable` toe te voegen.

```bash
docker run --name hbbs -p 21115:21115 -p 21116:21116 -p 21116:21116/udp -p 21118:21118 -v "$PWD/data:/root" -d rustdesk/rustdesk-server:latest hbbs -r <relay-server-ip[:port]> 
docker run --name hbbr -p 21117:21117 -p 21119:21119 -v "$PWD/data:/root" -d rustdesk/rustdesk-server:latest hbbr 
```

De `relay-server-ip` parameter is het IP adres (of dns naam) van de server waarop deze containers draaien. De **optionele** `port` parameter moet gebruikt worden als je een andere poort dan **21117** gebruikt voor `hbbr`.

U kunt ook docker-compose gebruiken, met deze configuratie als sjabloon:

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
      - ./data:/root
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
      - ./data:/root
    networks:
      - rustdesk-net
    restart: unless-stopped
```

Bewerk regel 16 om te verwijzen naar uw relais-server (degene die luistert op poort 21117). U kunt ook de inhoudsregels (L18 en L33) bewerken indien nodig.

(docker-compose erkenning gaat naar @lukebarone en @QuiGonLeong)

## S6-overlay gebaseerde bestanden

Deze bestanden (images) zijn gebouwd tegen `busybox:stable` met toevoeging van de binaries (zowel hbbr als hbbs) en [S6-overlay](https://github.com/just-containers/s6-overlay). Ze zijn beschikbaar op [Docker hub](https://hub.docker.com/r/rustdesk/rustdesk-server-s6/) met deze tags:

| architectuur | versie | image:tag |
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

Je wordt sterk aangeraden om het `multiarch` bestand (image) te gebruiken met de `major version` of `latest` tag.

De S6-overlay fungeert als supervisor en houdt beide processen draaiende, dus met dit bestand (image) is het niet nodig om twee aparte draaiende containers te hebben.

U kunt deze bestanden (images) direct starten via `docker run` met dit commando:

```bash
docker run --name rustdesk-server \ 
  --net=host \
  -e "RELAY=rustdeskrelay.example.com" \
  -e "ENCRYPTED_ONLY=1" \
  -v "$PWD/data:/data" -d rustdesk/rustdesk-server-s6:latest
```

of zonder `--net=host`, maar een directe P2P verbinding zal niet werken.

```bash
docker run --name rustdesk-server \
  -p 21115:21115 -p 21116:21116 -p 21116:21116/udp \
  -p 21117:21117 -p 21118:21118 -p 21119:21119 \
  -e "RELAY=rustdeskrelay.example.com" \
  -e "ENCRYPTED_ONLY=1" \
  -v "$PWD/data:/data" -d rustdesk/rustdesk-server-s6:latest
```

Of u kunt een docker-compose bestand gebruiken:

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

Voor dit container bestand (image) kunt u deze omgevingsvariabelen gebruiken, **naast** de variabelen in de volgende **ENV-variabelen** sectie:

| variabele | optioneel | beschrijving |
| --- | --- | --- |
| RELAY | no | het IP-adres/DNS-naam van de machine waarop deze container draait |
| ENCRYPTED_ONLY | yes | indien ingesteld op **"1"** wordt een niet-versleutelde verbinding niet geaccepteerd |
| KEY_PUB | yes | het openbare deel van het key paar |
| KEY_PRIV | yes | het private deel van het key paar |

### Geheim beheer in S6-overlay gebaseerde bestanden (images)

U kunt uiteraard het key paar bewaren in een docker volume, maar de optimale werkwijzen vertellen u om de keys niet op het bestandssysteem te schrijven; dus bieden we een paar opties.

Bij het opstarten van de container wordt de aanwezigheid van het key paar gecontroleerd (`/data/id_ed25519.pub` en `/data/id_ed25519`) en als een van deze keys niet bestaat, wordt deze opnieuw aangemaakt vanuit ENV variabelen of docker secrets.
Vervolgens wordt de geldigheid van het key paar gecontroleerd: indien publieke en private keys niet overeenkomen, stopt de container.
Als je geen keys opgeeft, zal `hbbs` er een voor je genereren en op de standaard locatie plaatsen.

#### Gebruik ENV om het key paar op te slaan

U kunt docker omgevingsvariabelen gebruiken om de keys op te slaan. Volg gewoon deze voorbeelden:

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

#### Gebruik Docker secrets om het key paar op te slaan

U kunt ook docker secrets gebruiken om de keys op te slaan.
Dit is handig als je **docker-compose** of **docker swarm** gebruikt.
Volg deze voorbeelden:

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

## Hoe maak je een key paar

Een key paar is nodig voor encryptie; u kunt het verstrekken, zoals eerder uitgelegd, maar u heeft een manier nodig om er een te maken.

U kunt dit commando gebruiken om een key paar te genereren:

```bash
/usr/bin/rustdesk-utils genkeypair
```

Als u het pakket `rustdesk-utils` niet op uw systeem hebt staan (of wilt), kunt u hetzelfde commando met docker uitvoeren:

```bash
docker run --rm --entrypoint /usr/bin/rustdesk-utils  rustdesk/rustdesk-server-s6:latest genkeypair
```

De uitvoer ziet er ongeveer zo uit:

```text
Public Key:  8BLLhtzUBU/XKAH4mep3p+IX4DSApe7qbAwNH9nv4yA=
Secret Key:  egAVd44u33ZEUIDTtksGcHeVeAwywarEdHmf99KM5ajwEsuG3NQFT9coAfiZ6nen4hfgNICl7upsDA0f2e/jIA==
```

## .deb pakketten

Voor elke binary zijn aparte .deb-pakketten beschikbaar, u kunt ze vinden in de [releases](https://github.com/rustdesk/rustdesk-server/releases).
Deze pakketten zijn bedoeld voor de volgende distributies:

- Ubuntu 22.04 LTS
- Ubuntu 20.04 LTS
- Ubuntu 18.04 LTS
- Debian 11 bullseye
- Debian 10 buster

## ENV variabelen

hbbs en hbbr kunnen worden geconfigureerd met deze ENV-variabelen.
U kunt de variabelen zoals gebruikelijk opgeven of een `.env` bestand gebruiken.

| variabele | binary | beschrijving |
| --- | --- | --- |
| ALWAYS_USE_RELAY | hbbs | indien ingesteld op **"Y"** wordt directe peer-verbinding niet toegestaan |
| DB_URL | hbbs | path voor database bestand |
| DOWNGRADE_START_CHECK | hbbr | vertraging (in seconden) voor downgrade-controle |
| DOWNGRADE_THRESHOLD | hbbr | drempel van downgrade controle (bit/ms) |
| KEY | hbbs/hbbr | indien ingesteld forceert dit het gebruik van een specifieke toets, indien ingesteld op **"_"** forceert dit het gebruik van een willekeurige toets |
| LIMIT_SPEED | hbbr | snelheidslimiet (in Mb/s) |
| PORT | hbbs/hbbr | luister-poort (21116 voor hbbs - 21117 voor hbbr) |
| RELAY_SERVERS | hbbs | IP-adres/DNS-naam van de machines waarop hbbr draait (gescheiden door komma) |
| RUST_LOG | all | debug-niveau instellen (error\|warn\|info\|debug\|trace) |
| SINGLE_BANDWIDTH | hbbr | maximale bandbreedte voor een enkele verbinding (in Mb/s) |
| TOTAL_BANDWIDTH | hbbr | maximale totale bandbreedte (in Mb/s) |
