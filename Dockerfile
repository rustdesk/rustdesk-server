# syntax=docker/dockerfile:1.7

############################
# 1) Build stage
############################
FROM rust:1.81-bookworm AS builder

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential pkg-config cmake clang perl \
    libssl-dev libsodium-dev protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY build.rs ./
COPY libs ./libs
COPY src ./src

RUN cargo build --release --bin hbbs --bin hbbr

############################
# 2) Runtime stage
############################
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates tzdata tini netcat-openbsd \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /data

COPY --from=builder /app/target/release/hbbs /usr/local/bin/hbbs
COPY --from=builder /app/target/release/hbbr /usr/local/bin/hbbr

ENV HBBS_PORT=21116
ENV HBBR_PORT=21117
ENV API_PORT=21114
ENV RELAY=127.0.0.1:21117
ENV ENCRYPTED_ONLY=0
ENV HBBS_ARGS=""
ENV HBBR_ARGS=""
ENV ADMIN_USERNAME=admin
ENV ADMIN_PASSWORD=admin123456
ENV ADMIN_JWT_SECRET=change-this-secret

EXPOSE 21114/tcp 21116/tcp 21116/udp 21117/tcp

HEALTHCHECK --interval=10s --timeout=3s --retries=6 CMD \
  nc -z 127.0.0.1 ${HBBS_PORT} && nc -z 127.0.0.1 ${HBBR_PORT} || exit 1

ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["/bin/sh", "-lc", "\
set -e; \
cd /data; \
PARAMS=''; [ \"$ENCRYPTED_ONLY\" = '1' ] && PARAMS='-k _'; \
hbbr -p ${HBBR_PORT} ${PARAMS} ${HBBR_ARGS} & \
HBBR_PID=$!; \
hbbs -p ${HBBS_PORT} -a ${API_PORT} -r ${RELAY} ${PARAMS} ${HBBS_ARGS} & \
HBBS_PID=$!; \
trap 'kill -TERM $HBBS_PID $HBBR_PID 2>/dev/null' TERM INT; \
wait -n $HBBS_PID $HBBR_PID; \
EXIT_CODE=$?; \
kill -TERM $HBBS_PID $HBBR_PID 2>/dev/null || true; \
wait || true; \
exit $EXIT_CODE \
"]
