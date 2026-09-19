#!/usr/bin/env bash
# Sets up a fresh clone: the pinned Rust toolchain, the pinned gate tools, and
# the git hooks. Safe to run again. It never installs rustup or mise itself.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

missing=0
if command -v rustup >/dev/null 2>&1; then
  echo "[ok] rustup"
else
  echo "error: rustup is not installed or not on PATH" >&2
  echo "  install it from https://rustup.rs, then rerun scripts/setup.sh" >&2
  missing=1
fi
if command -v mise >/dev/null 2>&1; then
  echo "[ok] mise"
else
  echo "error: mise is not installed or not on PATH" >&2
  echo "  install it from https://mise.jdx.dev/installing-mise.html, then rerun scripts/setup.sh" >&2
  missing=1
fi
if ((missing)); then
  exit 1
fi

# rustup and mise both resolve their pin files from the current directory.
cd "$REPO_ROOT"

# Before mise, whose cargo backend needs a working cargo. The fallback is for
# an older rustup, where `show active-toolchain` installs the pin.
echo "[..] Rust toolchain (rust-toolchain.toml)"
rustup toolchain install || rustup show active-toolchain
echo "[ok] Rust toolchain"

# Running this script is the decision to trust mise.toml.
echo "[..] gate tools (mise.toml)"
mise trust "$REPO_ROOT/mise.toml"
mise install
echo "[ok] gate tools"

"$REPO_ROOT/scripts/install-hooks.sh"

echo
echo "Setup complete. Run the full local gate with:"
echo "  cargo xtask pre-push"
