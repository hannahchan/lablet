#!/usr/bin/env bash
# Point this clone's git hooks at scripts/hooks, so the hooks are the versioned
# files and an update to them needs no reinstall. Run once per clone;
# scripts/setup.sh runs it for you. Safe to run again.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOOKS_PATH="scripts/hooks"

for hook in pre-commit pre-push; do
  if [[ ! -x "$REPO_ROOT/$HOOKS_PATH/$hook" ]]; then
    echo "error: $HOOKS_PATH/$hook is missing or not executable" >&2
    exit 1
  fi
done

# The path is relative, so git resolves it against the working tree the hook
# runs in: each worktree of this clone runs its own checkout's hooks. The
# setting lives in the clone's local config, never the global one.
git -C "$REPO_ROOT" config --local core.hooksPath "$HOOKS_PATH"

echo "[ok] git hooks: core.hooksPath = $HOOKS_PATH (pre-commit, pre-push)"
