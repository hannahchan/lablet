#!/usr/bin/env bash
# Copies what weaver reads from upstream into deps/: weaver clones a git
# dependency on every run and keeps no cache, so the registry names local
# paths. The `vendor` calls at the end of this file are the pins.
#
#   vendor.sh          replace deps/ with the pinned upstream files
#   vendor.sh --check  fetch the pins again and fail if deps/ differs
set -euo pipefail

case "${1:-}" in
  "") check=0 ;;
  --check) check=1 ;;
  *)
    echo "usage: $0 [--check]" >&2
    exit 2
    ;;
esac

DEPS="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/deps"
SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
STAGED="$SCRATCH/deps"

# vendor <name> <repository> <ref> <commit> <pathspec>...
# <ref> is what upstream calls <commit>. Only <commit> is fetched: a tag can
# move, and a fetch by commit can be shallow.
vendor() {
  local name=$1 repository=$2 ref=$3 commit=$4
  shift 4
  local tree="$STAGED/$name"
  git init -q "$tree"
  git -C "$tree" fetch -q --depth 1 "$repository" "$commit"
  # The copy stays byte-identical whatever the developer's autocrlf says.
  git -C "$tree" -c core.autocrlf=false checkout -q FETCH_HEAD -- "$@"
  {
    echo "Written by lablet/telemetry/vendor.sh, which holds the pin. Don't edit this tree."
    echo
    echo "repository: $repository"
    echo "ref: $ref"
    echo "commit: $commit"
    echo "committed: $(git -C "$tree" show -s --format=%cI FETCH_HEAD)"
    echo "copied:"
    printf '  %s\n' "$@"
  } >"$tree/SOURCES"
  rm -rf "$tree/.git"
}

vendor semantic-conventions \
  https://github.com/open-telemetry/semantic-conventions.git \
  v1.44.0 e10a930844c6951757a43b849d364f7d056ac32b \
  model LICENSE

vendor semantic-conventions-genai \
  https://github.com/open-telemetry/semantic-conventions-genai.git \
  main c88d504ab3d9879f8e50d3cc87e69775e11db234 \
  model LICENSE

# The GenAI registry names the core conventions by git URL, and weaver clones
# every git dependency it meets, vendored parent or not. This line is the one
# vendored byte that isn't upstream's. The grep fails the run if a new pin
# words the line differently.
manifest="$STAGED/semantic-conventions-genai/model/manifest.yaml"
core="lablet/telemetry/deps/semantic-conventions/model"
sed "s|\(registry_path: \)https://github.com/open-telemetry/semantic-conventions\.git@.*|\1$core|" \
  "$manifest" >"$manifest.local"
mv "$manifest.local" "$manifest"
grep -q "registry_path: $core\$" "$manifest"
echo "changed: model/manifest.yaml, where registry_path names $core" \
  >>"$STAGED/semantic-conventions-genai/SOURCES"

# Upstream's fixtures for its own policy and template tests aren't read here.
vendor weaver-packages \
  https://github.com/open-telemetry/opentelemetry-weaver-packages.git \
  main 587265869c37180940d3bae29d23ca385e6baa00 \
  policies/check/naming_conventions policies/check/stability templates/docs/markdown LICENSE \
  ':(exclude,glob)**/tests/**'

if ((check)); then
  if ! diff -r "$DEPS" "$STAGED"; then
    echo "error: lablet/telemetry/deps differs from upstream at the pinned commits" >&2
    echo "  restore it with: cargo xtask weaver vendor" >&2
    exit 1
  fi
  echo "lablet/telemetry/deps matches upstream at the pinned commits"
else
  rm -rf "$DEPS"
  mv "$STAGED" "$DEPS"
  echo "vendored into lablet/telemetry/deps; review the diff before committing"
fi
