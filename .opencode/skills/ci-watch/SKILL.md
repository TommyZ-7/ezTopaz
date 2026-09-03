---
name: CI Watch
description: Monitor GitHub Actions runs for a branch to completion with dynamic latest-run tracking. Use when waiting on CI or Release workflow results after a push or PR creation.
---

## Workflow

1. Identify target: `BRANCH` (e.g. `fix/xxx`) and `WF` (`CI` or `Release`).
2. shellツールの`background: true`で `scripts/ci-watch.sh <branch> [workflow]` を起動する
   (フォアグラウンドは120秒で切れるため不可)。

   ```bash
   .opencode/skills/ci-watch/scripts/ci-watch.sh <branch> <workflow> > /tmp/opencode/ci-watch.log 2>&1
   ```

   - `<workflow>` は `CI` または `Release`。途中経過は `tail /tmp/opencode/ci-watch.log` で確認。
   - コマンド文への `&`/`nohup` 付加は禁止。シェル内バックグラウンド化はツール管理外と
     なり、TUIのshell表示も完了通知も出ない (2026-09-03に実績あり)。
   - `export` と起動は別文にすること。`BRANCH=... cmd "$BRANCH"` の一行書きは禁止
     (`VAR=...` prefixは起動コマンドへのenvにしかならず、同行の `"$VAR"` 展開時点では
     空になる。スクリプト側にenvフォールバックはあるが当てにしないこと)。
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
