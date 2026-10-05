#!/bin/sh
# Generates each crate's typed telemetry into the spike crates, as xtask would.
set -eu
cd "$(dirname "$0")"
W=/Users/hannah.chan/.local/share/mise/installs/github-open-telemetry-weaver/0.26.1/weaver
OUT=${1:-.}
for pair in "application/run:crates/spike-run/src/telemetry" "apps/lablet:crates/spike-root/src/telemetry"; do
  dir=${pair%%:*}
  dest=$OUT/${pair#*:}
  rm -rf "$dest"
  "$W" registry generate --v2 --quiet \
    -r lablet/telemetry/registry -t lablet/telemetry/templates \
    --param crate_dir="$dir" --param target=lablet \
    rust-crate "$dest" 2>&1 | grep -v "not yet stable" | grep -v "^$" || true
  rustfmt --edition 2024 "$dest/mod.rs"
done
