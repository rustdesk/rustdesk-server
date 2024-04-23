<p align="center">
  <a href="#如何自行建置">自行建置</a> •
  <a href="#Docker-映像檔">Docker</a> •
  <a href="#基於-S6-overlay-的映象檔">S6-overlay</a> •
  <a href="#如何建立金鑰對">金鑰對</a> •
  <a href="#deb-套件">Debian</a> •
  <a href="#ENV-環境參數">環境參數</a><br>
  [<a href="README.md">English</a>] | [<a href="README-DE.md">Deutsch</a>] | [<a href="README-NL.md">Nederlands</a>] | [<a href="README-ZH.md">简体中文</a>]<br>
</p>

# RustDesk Server Program

[![build](https://github.com/rustdesk/rustdesk-server/actions/workflows/build.yaml/badge.svg)](https://github.com/rustdesk/rustdesk-server/actions/workflows/build.yaml)

[**下載**](https://github.com/rustdesk/rustdesk-server/releases)

[**說明文件**](https://rustdesk.com/docs/zh-tw/self-host/)

[**FAQ**](https://github.com/rustdesk/rustdesk/wiki/FAQ)

自行建置屬於您自己的 RustDesk 伺服器，它是免費的且開源。

## 如何自行建置

```bash
cargo build --release
```

在 target/release 中會產生三個可執行檔。

- hbbs - RustDesk ID/會合伺服器
- hbbr - RustDesk 中繼伺服器
- rustdesk-utils - RustDesk 命令行工具

您可以在 [releases](https://github.com/rustdesk/rustdesk-server/releases) 頁面上找到更新的執行檔。

如果您需要額外功能，[RustDesk 專業版伺服器](https://rustdesk.com/pricing.html) 或許更適合您。

如果您想開發自己的伺服器，[rustdesk-server-demo](https://github.com/rustdesk/rustdesk-server-demo) 可能是一個比這個倉庫更好、更簡單的開始。

## Docker 映像檔

Docker 映像檔會在每次 GitHub 發布時自動生成並發布。我們有兩種映像檔。

### Classic 映像檔

這些映像檔是基於 `ubuntu-20.04` 建置的，僅添加了兩個主要的執行檔（`hbbr` 和 `hbbs`）。它們可在 [Docker Hub](https://hub.docker.com/r/rustdesk/rustdesk-server/) 上取得，帶有以下tags：

| 架構    | image:tag                                 |
| ------- | ----------------------------------------- |
| amd64   | `rustdesk/rustdesk-server:latest`         |
| arm64v8 | `rustdesk/rustdesk-server:latest-arm64v8` |

您可以使用以下指令，直接透過 ``docker run`` 來啟動這些映像檔：

```bash
docker run --name hbbs --net=host -v "$PWD/data:/root" -d rustdesk/rustdesk-server:latest hbbs -r <relay-server-ip[:port]> 
docker run --name hbbr --net=host -v "$PWD/data:/root" -d rustdesk/rustdesk-server:latest hbbr 
```

或刪去 `--net=host`， 但 P2P 直接連線會無法運作。

對於使用 SELinux 的系統，需要將 ``/root`` 替換為 ``/root:z``，以便容器正確運行。或者，也可以通過添加選項 ``--security-opt label=disable`` 完全禁用 SELinux 容器隔離。

```bash
docker run --name hbbs -p 21115:21115 -p 21116:21116 -p 21116:21116/udp -p 21118:21118 -v "$PWD/data:/root" -d rustdesk/rustdesk-server:latest hbbs -r <relay-server-ip[:port]> 
docker run --name hbbr -p 21117:21117 -p 21119:21119 -v "$PWD/data:/root" -d rustdesk/rustdesk-server:latest hbbr 
```

`relay-server-ip` 參數是執行這些容器的伺服器的 IP 地址（或 DNS 名稱）。如果您為 `hbbr` 使用的端口不是 **21117**，則必須使用 **可選** 的 `port` 參數。

您也可以使用 docker-compose 使用這個設定做為範例：

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

請編輯第 16 行，將其指向您的中繼伺服器 （監聽端口 21117 那一個）。 如果需要的話，您也可以編輯 volume  (第 18 和 33 行)。

（感謝 @lukebarone 和 @QuiGonLeong 協助提供 docker-compose 的設定範例）

## 基於 S6-overlay 的映象檔

這些映象檔是針對 `busybox:stable` 建置的，並添加了執行檔（hbbr 和 hbbs）以及 [S6-overlay](https://github.com/just-containers/s6-overlay)。 它們在以及這些 tags 在 [Docker hub](https://hub.docker.com/r/rustdesk/rustdesk-server-s6/) 可用：

| 架構      | version | image:tag                                    |
| --------- | ------- | -------------------------------------------- |
| multiarch | latest  | `rustdesk/rustdesk-server-s6:latest`         |
| amd64     | latest  | `rustdesk/rustdesk-server-s6:latest-amd64`   |
| i386      | latest  | `rustdesk/rustdesk-server-s6:latest-i386`    |
| arm64v8   | latest  | `rustdesk/rustdesk-server-s6:latest-arm64v8` |
| armv7     | latest  | `rustdesk/rustdesk-server-s6:latest-armv7`   |
| multiarch | 2       | `rustdesk/rustdesk-server-s6:2`              |
| amd64     | 2       | `rustdesk/rustdesk-server-s6:2-amd64`        |
| i386      | 2       | `rustdesk/rustdesk-server-s6:2-i386`         |
| arm64v8   | 2       | `rustdesk/rustdesk-server-s6:2-arm64v8`      |
| armv7     | 2       | `rustdesk/rustdesk-server-s6:2-armv7`        |
| multiarch | 2.0.0   | `rustdesk/rustdesk-server-s6:2.0.0`          |
| amd64     | 2.0.0   | `rustdesk/rustdesk-server-s6:2.0.0-amd64`    |
| i386      | 2.0.0   | `rustdesk/rustdesk-server-s6:2.0.0-i386`     |
| arm64v8   | 2.0.0   | `rustdesk/rustdesk-server-s6:2.0.0-arm64v8`  |
| armv7     | 2.0.0   | `rustdesk/rustdesk-server-s6:2.0.0-armv7`    |

強烈建議您使用 `multiarch` 映象檔 可以選擇使用 `major version` 或 `latest` tags。

S6-overlay 在此充當監督程序，保持兩個進程運行，因此使用此映象檔，您無需運行兩個獨立的容器。

您可以直接使用以下命令使用 `docker run` 來啟動這個映象檔：

```bash
docker run --name rustdesk-server \ 
  --net=host \
  -e "RELAY=rustdeskrelay.example.com" \
  -e "ENCRYPTED_ONLY=1" \
  -v "$PWD/data:/data" -d rustdesk/rustdesk-server-s6:latest
```

或刪去 `--net=host`， 但 P2P 直接連線會無法運作。

```bash
docker run --name rustdesk-server \
  -p 21115:21115 -p 21116:21116 -p 21116:21116/udp \
  -p 21117:21117 -p 21118:21118 -p 21119:21119 \
  -e "RELAY=rustdeskrelay.example.com" \
  -e "ENCRYPTED_ONLY=1" \
  -v "$PWD/data:/data" -d rustdesk/rustdesk-server-s6:latest
```

或是您可以使用 docker-compose 文件:

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

對於此容器映象檔，您可以使用這些環境變數，**除了**以下**環境變數**部分指定的那些。

| 環境變數       | 是否可選 | 敘述                                       |
| -------------- | -------- | ------------------------------------------ |
| RELAY          | 否       | 運行此容器的機器的 IP 地址/ DNS 名稱       |
| ENCRYPTED_ONLY | 是       | 如果設置為 **"1"**，將不接受未加密的連接。 |
| KEY_PUB        | 是       | 金鑰對中的公鑰（Public Key）               |
| KEY_PRIV       | 是       | 金鑰對中的私鑰（Private Key）               |

###  在基於 S6-overlay 的 Secret 管理

您可以將金鑰對保存在 Docker volume 中，但最佳實踐建議不要將金鑰寫入文件系統；因此，我們提供了一些選項。

在容器啟動時，會檢查金鑰對的是否存在（`/data/id_ed25519.pub` 和 `/data/id_ed25519`），如果其中一個金鑰不存在，則會從環境變數或 Docker Secret 重新生成它。
然後檢查金鑰對的有效性：如果公鑰和私鑰不匹配，容器將停止運行。
如果您未提供金鑰，`hbbs` 將為您產生一個，並將其放置在默認位置。

#### 使用 ENV 存儲金鑰對

您可以使用 Docker 環境變數來儲存金鑰。只需按照以下範例操作：

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

#### 使用 Docker Secret 來儲存金鑰對

您還可以使用 Docker Secret來儲存金鑰。
如果您使用 **docker-compose** 或 **docker swarm**，這很有用。
只需按照以下示例操作：

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

## 如何建立金鑰對

加密需要一對金鑰；您可以按照前面所述提供它，但需要一種生成金鑰對的方法。

您可以使用以下命令生成一對金鑰：

```bash
/usr/bin/rustdesk-utils genkeypair
```

如果您沒有（或不想）在系統上安裝 `rustdesk-utils` 套件，您可以使用 Docker執行相同的命令：

```bash
docker run --rm --entrypoint /usr/bin/rustdesk-utils  rustdesk/rustdesk-server-s6:latest genkeypair
```

輸出將類似於以下內容：

```text
Public Key:  8BLLhtzUBU/XKAH4mep3p+IX4DSApe7qbAwNH9nv4yA=
Secret Key:  egAVd44u33ZEUIDTtksGcHeVeAwywarEdHmf99KM5ajwEsuG3NQFT9coAfiZ6nen4hfgNICl7upsDA0f2e/jIA==
```

## .deb 套件

每個執行檔都有單獨的 .deb 套件可供使用，您可以在 [releases](https://github.com/rustdesk/rustdesk-server/releases) 中找到它們。
這些套件適用於以下發行版：

- Ubuntu 22.04 LTS
- Ubuntu 20.04 LTS
- Ubuntu 18.04 LTS
- Debian 11 bullseye
- Debian 10 buster

## ENV 環境參數

可以使用這些 ENV 參數來配置 hbbs 和 hbbr。
您可以像往常一樣指定參數，或者使用 .env 文件。

| 參數                  | 執行檔    | 敘述                                                                 |
| --------------------- | --------- | -------------------------------------------------------------------- |
| ALWAYS_USE_RELAY      | hbbs      | 如果設為 **"Y"**，禁止直接點對點連接                                 |
| DB_URL                | hbbs      | 資料庫的路徑                                                         |
| DOWNGRADE_START_CHECK | hbbr      | 降級檢查之前的延遲時間（以秒為單位）                                 |
| DOWNGRADE_THRESHOLD   | hbbr      | 降級檢查的閾值（bit/ms）                                             |
| KEY                   | hbbs/hbbr | 如果設置了，將強制使用特定金鑰，如果設為 **"_"**，則強制使用任何金鑰 |
| LIMIT_SPEED           | hbbr      | 速度限制（以Mb/s為單位）                                             |
| PORT                  | hbbs/hbbr | 監聽端口（hbbs為21116，hbbr為21117）                                 |
| RELAY_SERVERS         | hbbs      | 運行hbbr的機器的IP地址/DNS名稱（用逗號分隔）                         |
| RUST_LOG              | all       | 設定 debug level (error\|warn\|info\|debug\|trace)                   |
| SINGLE_BANDWIDTH      | hbbr      | 單個連接的最大頻寬（以Mb/s為單位）                                   |
| TOTAL_BANDWIDTH       | hbbr      | 最大總頻寬（以Mb/s為單位）                                           |