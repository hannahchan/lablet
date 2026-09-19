#!/usr/bin/env bash
# Points this clone's git hooks at scripts/hooks, so an update to the versioned
# files needs no reinstall. Safe to run again.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOOKS_PATH="scripts/hooks"

for hook in pre-commit pre-push; do
  if [[ ! -x "$REPO_ROOT/$HOOKS_PATH/$hook" ]]; then
    echo "error: $HOOKS_PATH/$hook is missing or not executable" >&2
    exit 1
  fi
done

# A relative path, so each worktree of this clone runs its own checkout's hooks.
git -C "$REPO_ROOT" config --local core.hooksPath "$HOOKS_PATH"

echo "[ok] git hooks: core.hooksPath = $HOOKS_PATH (pre-commit, pre-push)"
