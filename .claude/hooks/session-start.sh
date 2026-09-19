#!/usr/bin/env bash
# Tells a new Claude Code session where it stands before it commits anything.
# Advisory only: it never fails the session.
set -uo pipefail

REPO="${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null || true)}"
[ -n "$REPO" ] || exit 0

branch=$(git -C "$REPO" branch --show-current 2>/dev/null || true)
if [ "$branch" = "main" ]; then
  echo "NOTE: on main. Work on a branch; fast-forward main only when a logical piece passes cargo xtask pre-push."
elif [ -n "$branch" ] && git -C "$REPO" rev-parse -q --verify origin/main >/dev/null 2>&1; then
  ahead=$(git -C "$REPO" log --oneline origin/main..HEAD 2>/dev/null || true)
  behind=$(git -C "$REPO" rev-list --count HEAD..origin/main 2>/dev/null || echo 0)
  upstream=$(git -C "$REPO" rev-parse --abbrev-ref '@{upstream}' 2>/dev/null || true)
  if [ -n "$ahead" ]; then
    echo "NOTE: $branch has commits that aren't on origin/main (fine if this work made them):"
    echo "$ahead"
  fi
  if [ "$behind" -gt 0 ]; then
    echo "WARNING: $branch is $behind commit(s) behind origin/main, so main won't fast-forward to it. Rebase first."
  fi
  if [ -n "$upstream" ] && [ "$upstream" != "origin/$branch" ] && [ "$upstream" != "origin/main" ]; then
    echo "WARNING: $branch tracks $upstream; a plain push would land there."
  fi
fi

if [ "$(git -C "$REPO" config --get core.hooksPath || true)" != "scripts/hooks" ]; then
  echo "NOTE: the git hooks aren't installed in this clone. Run: cargo xtask setup"
fi
