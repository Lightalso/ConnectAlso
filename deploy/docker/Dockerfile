# ── Build stage ──
FROM rust:1.85-slim-bookworm AS builder

RUN apt-get update && apt-get install -y \
    pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy workspace manifest
COPY Cargo.toml ./

# Copy crate manifests (for dependency caching)
COPY crates/core/Cargo.toml          crates/core/
COPY crates/crypto/Cargo.toml        crates/crypto/
COPY crates/tunnel/Cargo.toml        crates/tunnel/
COPY crates/nat/Cargo.toml           crates/nat/
COPY crates/platform/Cargo.toml      crates/platform/
COPY crates/relay-proto/Cargo.toml   crates/relay-proto/

COPY services/control/Cargo.toml     services/control/
COPY services/relay/Cargo.toml       services/relay/
COPY services/stun/Cargo.toml        services/stun/

COPY apps/daemon/Cargo.toml          apps/daemon/
COPY apps/cli/Cargo.toml             apps/cli/
COPY apps/desktop/Cargo.toml         apps/desktop/

# Create dummy source for dependency pre-fetch
RUN mkdir -p crates/core/src crates/crypto/src crates/tunnel/src \
    crates/nat/src crates/platform/src crates/relay-proto/src \
    services/control/src services/relay/src services/stun/src \
    apps/daemon/src apps/cli/src apps/desktop/src

RUN echo 'pub fn dummy() {}' > crates/core/src/lib.rs \
    && echo 'pub fn dummy() {}' > crates/crypto/src/lib.rs \
    && echo 'pub fn dummy() {}' > crates/tunnel/src/lib.rs \
    && echo 'pub fn dummy() {}' > crates/nat/src/lib.rs \
    && echo 'pub fn dummy() {}' > crates/platform/src/lib.rs \
    && echo 'pub fn dummy() {}' > crates/relay-proto/src/lib.rs \
    && echo 'fn main() {}' > services/control/src/main.rs \
    && echo 'fn main() {}' > services/relay/src/main.rs \
    && echo 'fn main() {}' > services/stun/src/main.rs \
    && echo 'fn main() {}' > apps/daemon/src/main.rs \
    && echo 'fn main() {}' > apps/cli/src/main.rs \
    && echo 'fn main() {}' > apps/desktop/src/main.rs

# Pre-fetch and compile dependencies
RUN cargo build --release \
    -p connectalso-control \
    -p connectalso-relay \
    -p connectalso-stun \
    -p connectalso-daemon \
    -p connectalso-cli \
    || true

# Copy actual source
COPY crates/ crates/
COPY services/ services/
COPY apps/ apps/

# Invalidate cargo cache for our crates
RUN find crates services apps -name "*.rs" -exec touch {} \;

# Build all binaries
RUN cargo build --release \
    -p connectalso-control \
    -p connectalso-relay \
    -p connectalso-stun \
    -p connectalso-daemon \
    -p connectalso-cli

# ── Runtime stage ──
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Copy binaries
COPY --from=builder /build/target/release/connectalso-control  /usr/local/bin/
COPY --from=builder /build/target/release/connectalso-relay    /usr/local/bin/
COPY --from=builder /build/target/release/connectalso-stun     /usr/local/bin/
COPY --from=builder /build/target/release/connectalso-daemon   /usr/local/bin/
COPY --from=builder /build/target/release/connectalso          /usr/local/bin/

# Data directory
RUN mkdir -p /var/lib/connectalso && chmod 777 /var/lib/connectalso

COPY deploy/docker/entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

ENTRYPOINT ["/entrypoint.sh"]
