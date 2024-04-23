<p align="center">
  <a href="#manuelles-erstellen">Erstellen</a> •
  <a href="#docker-image">Docker</a> •
  <a href="#s6-overlay-basierte-images">S6-Overlay</a> •
  <a href="#ein-schlüsselpaar-erstellen">Schlüsselpaar</a> •
  <a href="#debian-pakete">Debian-Pakete</a> •
  <a href="#umgebungsvariablen">Umgebungsvariablen</a><br>
  [<a href="README.md">English</a>] | [<a href="README-NL.md">Nederlands</a>] | [<a href="README-TW.md">繁體中文</a>] | [<a href="README-ZH.md">简体中文</a>]<br>
</p>

# RustDesk Server-Programm

[![build](https://github.com/rustdesk/rustdesk-server/actions/workflows/build.yaml/badge.svg)](https://github.com/rustdesk/rustdesk-server/actions/workflows/build.yaml)

[**Herunterladen**](https://github.com/rustdesk/rustdesk-server/releases)

[**Handbuch**](https://rustdesk.com/docs/de/self-host/)

[**FAQ**](https://github.com/rustdesk/rustdesk/wiki/FAQ)

Hosten Sie Ihren eigenen RustDesk-Server selbst, er ist kostenlos und quelloffen.

## Manuelles Erstellen

```bash
cargo build --release
```

In target/release werden drei ausführbare Dateien erzeugt.

- hbbs - RustDesk ID/Rendezvous-Server
- hbbr - RustDesk Relay-Server
- rustdesk-utils - RustDesk CLI-Utilities

[Hier](https://github.com/rustdesk/rustdesk-server/releases) finden Sie aktualisierte Binärdateien.

Wenn Sie Ihren eigenen Server entwickeln wollen, könnte [rustdesk-server-demo](https://github.com/rustdesk/rustdesk-server-demo) ein besserer und einfacherer Start für Sie sein als dieses Repository.

## Docker-Image

Docker-Images werden automatisch generiert und bei jedem Github-Release veröffentlicht. Wir haben 2 Arten von Images.

### Klassisches Image

Diese Images sind mit `Ubuntu 20.04` gebaut, mit dem Zusatz der wichtigen Binärdateien (`hbbr` und `hbbs`). Sie sind auf [Docker hub](https://hub.docker.com/r/rustdesk/rustdesk-server/) mit diesen Tags verfügbar:

| Architektur | Image:Tag |
| --- | --- |
| amd64 | `rustdesk/rustdesk-server:latest` |
| arm64v8 | `rustdesk/rustdesk-server:latest-arm64v8` |

Sie können diese Images direkt mit `docker run` mit diesen Befehlen starten:

```bash
docker run --name hbbs --net=host -v "$PWD/data:/root" -d rustdesk/rustdesk-server:latest hbbs -r <relay-server-ip[:port]> 
docker run --name hbbr --net=host -v "$PWD/data:/root" -d rustdesk/rustdesk-server:latest hbbr 
```

Oder ohne `--net=host`, aber die P2P-Direktverbindung kann dann nicht funktionieren.

Bei Systemen, die SELinux verwenden, muss `/root` durch `/root:z` ersetzt werden, damit die Container korrekt laufen. Alternativ kann die SELinux-Containertrennung durch Hinzufügen der Option `--security-opt label=disable` vollständig deaktiviert werden.

```bash
docker run --name hbbs -p 21115:21115 -p 21116:21116 -p 21116:21116/udp -p 21118:21118 -v "$PWD/data:/root" -d rustdesk/rustdesk-server:latest hbbs -r <relay-server-ip[:port]> 
docker run --name hbbr -p 21117:21117 -p 21119:21119 -v "$PWD/data:/root" -d rustdesk/rustdesk-server:latest hbbr 
```

Der Parameter `relay-server-ip` ist die IP-Adresse (oder der DNS-Name) des Servers, auf dem diese Container laufen. Der **optionale** Parameter `port` muss verwendet werden, wenn Sie einen anderen Port als **21117** für `hbbr` verwenden.

Sie können auch Docker Compose verwenden, wobei diese Konfiguration als Vorlage dient:

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

Bearbeiten Sie Zeile 16 so, dass sie auf Ihren Relay-Server verweist (den, der am Port 21117 lauscht). Sie können auch die Zeilen für die Volumes (Zeile 18 und 33) bearbeiten, wenn Sie dies wünschen.

(Die Anerkennung für Docker Compose geht an @lukebarone und @QuiGonLeong.)

## S6-Overlay-basierte Images

Diese Images sind mit `busybox:stable` gebaut, mit dem Zusatz Binärdateien (sowohl hbbr als auch hbbs) und [S6-overlay](https://github.com/just-containers/s6-overlay). Sie sind auf [Docker hub](https://hub.docker.com/r/rustdesk/rustdesk-server-s6/) mit diesen Tags verfügbar:

| Architektur | Version | Image:Tag |
| --- | --- | --- |
| multiarch | neueste | `rustdesk/rustdesk-server-s6:latest` |
| amd64 | neueste | `rustdesk/rustdesk-server-s6:latest-amd64` |
| i386 | neueste | `rustdesk/rustdesk-server-s6:latest-i386` |
| arm64v8 | neueste | `rustdesk/rustdesk-server-s6:latest-arm64v8` |
| armv7 | neueste | `rustdesk/rustdesk-server-s6:latest-armv7` |
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

Es wird dringend empfohlen, das Image `multiarch` entweder mit dem Tag `major version` oder `latest` zu verwenden.

Das S6-Overlay fungiert als Supervisor und hält beide Prozesse am Laufen, sodass bei diesem Image keine zwei separaten Container benötigt werden.

Sie können diese Images direkt mit `docker run` mit diesem Befehl starten:

```bash
docker run --name rustdesk-server \ 
  --net=host \
  -e "RELAY=rustdeskrelay.example.com" \
  -e "ENCRYPTED_ONLY=1" \
  -v "$PWD/data:/data" -d rustdesk/rustdesk-server-s6:latest
```

oder ohne `--net=host`, aber die P2P-Direktverbindung kann dann nicht funktionieren.

```bash
docker run --name rustdesk-server \
  -p 21115:21115 -p 21116:21116 -p 21116:21116/udp \
  -p 21117:21117 -p 21118:21118 -p 21119:21119 \
  -e "RELAY=rustdeskrelay.example.com" \
  -e "ENCRYPTED_ONLY=1" \
  -v "$PWD/data:/data" -d rustdesk/rustdesk-server-s6:latest
```

Oder Sie können eine Docker Compose-Datei verwenden:

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

Für dieses Container-Image können Sie diese Umgebungsvariablen verwenden, **zusätzlich** zu den im Abschnitt **Umgebungsvariablen** angegebenen Variablen:

| Variable | optional | Beschreibung |
| --- | --- | --- |
| RELAY | nein | IP-Adresse/DNS-Name des Rechners, auf dem dieser Container läuft |
| ENCRYPTED_ONLY | ja | Wenn auf **1** gesetzt, wird eine unverschlüsselte Verbindung nicht akzeptiert |
| KEY_PUB | ja | Öffentlicher Teil des Schlüsselpaares |
| KEY_PRIV | ja | Privater Teil des Schlüsselpaares |

### Verwaltung von Geheimnissen in S6-Overlay-basierten Images

Sie können das Schlüsselpaar natürlich in einem Docker-Volume aufbewahren, aber empfehlenswert ist, die Schlüssel nicht in das Dateisystem zu schreiben.

Beim Start des Containers wird das Vorhandensein des Schlüsselpaares geprüft (`/data/id_ed25519.pub` und `/data/id_ed25519`). Wenn einer dieser Schlüssel nicht existiert, wird er aus den Umgebungsvariablen oder den Docker-Geheimnissen neu erstellt.
Dann wird die Gültigkeit des Schlüsselpaares überprüft: Wenn öffentlicher und privater Schlüssel nicht übereinstimmen, wird der Container angehalten.
Wenn Sie keine Schlüssel angeben, erzeugt `hbbs` einen für Sie und legt ihn am Standardspeicherort ab.

#### Umgebungsvariablen zum Speichern des Schlüsselpaars verwenden

Sie können Docker-Umgebungsvariablen verwenden, um die Schlüssel zu speichern. Folgen Sie einfach diesen Beispielen:

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

#### Docker-Geheimnisse zum Speichern des Schlüsselpaars verwenden

Sie können alternativ auch Docker-Geheimnisse verwenden, um die Schlüssel zu speichern.
Dies ist nützlich, wenn Sie **Docker Compose** oder **Docker Swarm** verwenden.
Folgen Sie einfach diesem Beispiel:

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

## Ein Schlüsselpaar erstellen

Für die Verschlüsselung wird ein Schlüsselpaar benötigt, das Sie bereitstellen können, aber Sie benötigen eine Möglichkeit, es zu erstellen.

Mit diesem Befehl können Sie ein Schlüsselpaar erzeugen:

```bash
/usr/bin/rustdesk-utils genkeypair
```

Wenn Sie das Paket `rustdesk-utils` nicht auf Ihrem System installiert haben (oder dies nicht wollen), können Sie den gleichen Befehl mit Docker aufrufen:

```bash
docker run --rm --entrypoint /usr/bin/rustdesk-utils  rustdesk/rustdesk-server-s6:latest genkeypair
```

Die Ausgabe sieht dann etwa so aus:

```text
Public Key:  8BLLhtzUBU/XKAH4mep3p+IX4DSApe7qbAwNH9nv4yA=
Secret Key:  egAVd44u33ZEUIDTtksGcHeVeAwywarEdHmf99KM5ajwEsuG3NQFT9coAfiZ6nen4hfgNICl7upsDA0f2e/jIA==
```

## Debian-Pakete

Für jede Binärdatei stehen separate Debian-Pakete zur Verfügung, die Sie in [Releases](https://github.com/rustdesk/rustdesk-server/releases) finden können.
Diese Pakete sind für die folgenden Distributionen gedacht:

- Ubuntu 22.04 LTS
- Ubuntu 20.04 LTS
- Ubuntu 18.04 LTS
- Debian 11 Bullseye
- Debian 10 Buster

## Umgebungsvariablen

hbbs und hbbr können mit diesen Umgebungsvariablen konfiguriert werden.
Sie können die Variablen wie üblich angeben oder eine `.env`-Datei verwenden.

| Variable | Binärdatei | Beschreibung |
| --- | --- | --- |
| ALWAYS_USE_RELAY | hbbs | Wenn auf **Y** gesetzt, wird eine direkte Verbindung nicht zugelassen. |
| DB_URL | hbbs | Pfad für die Datenbankdatei |
| DOWNGRADE_START_CHECK | hbbr | Verzögerung (in Sekunden) vor der Downgrade-Prüfung |
| DOWNGRADE_THRESHOLD | hbbr | Schwellenwert der Downgrade-Prüfung (Bit/ms)) |
| KEY | hbbs/hbbr | Wenn gesetzt, wird die Verwendung eines bestimmten Schlüssels erzwungen. Wenn auf **_** gesetzt, wird die Verwendung eines beliebigen Schlüssels erzwungen. |
| LIMIT_SPEED | hbbr | Höchstgeschwindigkeit (in Mb/s) |
| PORT | hbbs/hbbr | Lauschender Port (21116 für hbbs - 21117 für hbbr) |
| RELAY_SERVERS | hbbs | IP-Adresse/DNS-Name der Rechner, auf denen hbbr läuft (durch Komma getrennt) |
| RUST_LOG | all | Debug-Level einstellen (error\|warn\|info\|debug\|trace) |
| SINGLE_BANDWIDTH | hbbr | Maximale Bandbreite für eine einzelne Verbindung (in Mb/s) |
| TOTAL_BANDWIDTH | hbbr | Maximale Gesamtbandbreite (in Mb/s) |
