#!/usr/bin/env bash
# One command for a fresh clone: the pinned Rust toolchain, the pinned gate
# tools, and the git hooks. Safe to run again; every step is a no-op when it
# has already been done.
#
# It installs nothing system-wide and never installs rustup or mise itself:
# those two are yours to install, and the messages below say where from.
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

# The toolchain comes first because mise's cargo backend needs a working cargo.
# `show active-toolchain` installs the pin from rust-toolchain.toml when it is
# absent; the fallback covers RUSTUP_AUTO_INSTALL=0 and needs rustup 1.28+.
echo "[..] Rust toolchain (rust-toolchain.toml)"
rustup show active-toolchain || rustup toolchain install
echo "[ok] Rust toolchain"

# mise reads a project's mise.toml only once it is trusted; running this
# script is that decision.
echo "[..] gate tools (mise.toml)"
mise trust "$REPO_ROOT/mise.toml"
mise install
echo "[ok] gate tools"

"$REPO_ROOT/scripts/install-hooks.sh"

echo
echo "Setup complete. Run the full local gate with:"
echo "  mise exec -- cargo xtask pre-push"
