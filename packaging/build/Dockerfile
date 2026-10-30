# The system release binaries are built on. Debian 13 has the GTK and libadwaita the app needs and
# an older C library than rolling distributions, and a program runs on C libraries newer than the
# one it was built against but not on older ones. That makes this the oldest usable base.
FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential pkg-config libgtk-4-dev libadwaita-1-dev rustup ca-certificates curl
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:/usr/bin:/bin
COPY rust-toolchain.toml /tmp/rust-toolchain.toml
RUN mkdir -p /opt/cargo/bin \
    && rustup toolchain install "$(sed -n 's/^channel = "\(.*\)"/\1/p' /tmp/rust-toolchain.toml)" --profile minimal \
    && chmod -R a+rX /opt/rustup /opt/cargo
