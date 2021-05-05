FROM ubuntu:20.04
COPY target/release/hbbs /usr/bin/hbbs
COPY target/release/hbbr /usr/bin/hbbr
WORKDIR /root
