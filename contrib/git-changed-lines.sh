#!/usr/bin/env bash
#
# git-changed-lines.sh — emit a manifest of changed `.py` line ranges
# suitable for `zorilla check --changed-lines`.
#
# Usage:
#   contrib/git-changed-lines.sh [BASE]
#
# BASE defaults to `origin/main`. The script walks the unified diff
# between BASE and HEAD (added/modified lines only), groups hunks by
# file, and prints one entry per file:
#
#   tests/test_orders.py:5-12,40-55
#   tests/test_returns.py:1-3
#
# Pure deletions (hunks with no `+` lines) are skipped — there's nothing
# to lint on a deleted line. Whole-file additions emit as range entries
# rather than bare paths; for a brand-new file the helper still produces
# a range covering the file, which is acceptable.
#
# Pipe into zorilla:
#   contrib/git-changed-lines.sh origin/main | zorilla check --changed-lines -

set -euo pipefail
LC_ALL=C
BASE="${1:-origin/main}"

git diff --unified=0 --no-color "${BASE}...HEAD" -- '*.py' \
  | awk '
      /^\+\+\+ b\// { file = substr($0, 7); next }
      /^@@/ {
        if (match($0, /\+([0-9]+)(,([0-9]+))?/, m) == 0) next
        start = m[1]; len = (m[3] == "" ? 1 : m[3])
        if (len == 0) next  # pure deletion
        end = start + len - 1
        ranges[file] = ranges[file] (ranges[file] ? "," : "") start "-" end
      }
      END { for (f in ranges) print f ":" ranges[f] }
  '
