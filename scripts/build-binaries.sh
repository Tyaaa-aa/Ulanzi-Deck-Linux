#!/usr/bin/env bash
# Build the release binaries on Debian 13, into target/container/release. Built here instead, they
# would need this computer's C library and refuse to start on anything older. Needs Docker.
set -euo pipefail
cd "$(dirname "$0")/.."
toolchain=$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)
docker build -q -t ulanzi-deck-build -f packaging/build/Dockerfile . >/dev/null
docker run --rm -u "$(id -u):$(id -g)" -v "$PWD:/src" -w /src \
  -e HOME=/tmp -e CARGO_HOME=/src/target/container/cargo-home -e PATH=/opt/cargo/bin:/usr/bin:/bin \
  -e CARGO_TARGET_DIR=/src/target/container -e RUSTUP_TOOLCHAIN="$toolchain" \
  ulanzi-deck-build cargo build --release --locked --bins
