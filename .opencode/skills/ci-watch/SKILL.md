---
name: CI Watch
description: Monitor GitHub Actions runs for a branch to completion with dynamic latest-run tracking. Use when waiting on CI or Release workflow results after a push or PR creation.
---

## Workflow

1. Identify target: `BRANCH` (e.g. `fix/xxx`) and `WF` (`CI` or `Release`).
2. Run `scripts/ci-watch.sh <branch> [workflow]` **in background** (foreground shells time out after 120s).
   Launch exactly as follows (two statements; do NOT one-line it):

   ```bash
   export BRANCH=<branch> WF=<workflow>  # WF: CI or Release
   nohup .opencode/skills/ci-watch/scripts/ci-watch.sh "$BRANCH" "$WF" > /tmp/opencode/ci-watch.log 2>&1 &
   ```

   ⚠️ `BRANCH=... nohup ... "$BRANCH" &` の一行書きは禁止。
   `VAR=...` prefixは`nohup`へのenvにしかならず、同行の`"$VAR"`展開時点では
   空のためusage終了する (2026-09-03に実績あり).
3. On completion notification, check the final summary and act:
   - success → merge flow (`gh pr merge <n> --merge`, switch to `main`, pull).
   - failure → fetch logs, analyze, fix on the branch.
4. Judgment on merge/fix stays centralized on notification; monitors are read-only.

## Rules (ezTopaz)

- `gh run watch` is unusable here (403 on annotations) → polling only.
- Never pin the run ID at start. A new push regenerates the run and the old
  one ends as `cancelled`; re-resolve the latest run on every poll.
- `gh pr checks` may 403 with limited tokens; `gh run list`/`gh run view` work.
- Details and recovery: `AGENTS.md` §5/§6.
