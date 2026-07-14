FROM rust:1.85-slim-bookworm

RUN apt-get update && apt-get install -y \
    iptables \
    iproute2 \
    iputils-ping \
    tcpdump \
    netcat-openbsd \
    dnsutils \
    curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# iptables requires NET_ADMIN capability at runtime
# The container must be started with --cap-add=NET_ADMIN
