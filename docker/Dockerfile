FROM rust:alpine AS builder

RUN \
  apk -U add musl-dev git file make && \
  git clone --depth=1 https://github.com/rustdesk/rustdesk-server.git /src && \
  cd /src && \
  cargo build -r --manifest-path /src/Cargo.toml

FROM busybox:stable

ARG S6_OVERLAY_VERSION=3.1.0.1
ADD https://github.com/just-containers/s6-overlay/releases/download/v${S6_OVERLAY_VERSION}/s6-overlay-noarch.tar.xz /tmp
ADD https://github.com/just-containers/s6-overlay/releases/download/v${S6_OVERLAY_VERSION}/s6-overlay-x86_64.tar.xz /tmp
RUN \
  tar -C / -Jxpf /tmp/s6-overlay-noarch.tar.xz && \
  tar -C / -Jxpf /tmp/s6-overlay-x86_64.tar.xz && \
  rm /tmp/s6-overlay*.tar.xz
  
COPY rootfs /
COPY --from=builder /src/target/release/hbbr /usr/bin/hbbr
COPY --from=builder /src/target/release/hbbs /usr/bin/hbbs
COPY healthcheck.sh /usr/bin/healthcheck.sh

ENV RELAY relay.example.com

EXPOSE 21115 21116 21116/udp 21117 21118 21119

HEALTHCHECK --interval=10s --timeout=5s CMD /usr/bin/healthcheck.sh

WORKDIR /data

VOLUME /data

ENTRYPOINT ["/init"]
