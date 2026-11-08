#!/usr/bin/env bash
# Builds the Debian package target/x86_64-unknown-linux-musl/debian/opensecagent_<version>-1_amd64.deb from the
# static binary. Extra arguments go to cargo-deb (e.g. --no-build after a release build).
# Needs cargo-deb (cargo install cargo-deb --locked) and the musl target (rustup target add x86_64-unknown-linux-musl).
# Only on request: test packages are built when asked for, releases only when announced.
set -euo pipefail

cd "$(dirname "$0")/.."
command -v cargo-deb >/dev/null || { echo "cargo-deb is missing: cargo install cargo-deb --locked" >&2; exit 1; }
if grep -n /usr/local packaging/linux/*.service packaging/linux/*.desktop packaging/linux/*.policy; then
  echo "error: paths under /usr/local left in the package files above" >&2
  exit 1
fi
cargo deb --target x86_64-unknown-linux-musl "$@"
