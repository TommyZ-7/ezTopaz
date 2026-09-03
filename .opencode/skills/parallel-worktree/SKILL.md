---
name: Parallel Worktree
description: Work on another branch in a separate session without disturbing the current checkout. Use when starting parallel work while CI runs or another session is active.
---

## Workflow

1. Create one worktree per session/branch (sibling of the repo root):
   `git worktree add ../ezTopaz-<suffix> -b <branch>`
   For an existing branch: `git worktree add ../ezTopaz-<suffix> <branch>`
2. Open the new session in the new worktree directory.
3. Keep a 1:1 session-to-branch mapping. Never check out the same branch in
   two worktrees (git forbids it) and never switch branches in a worktree
   owned by another session. Branch create/switch shell commands are denied
   by project `opencode.json` permissions (except switching to `main` and
   path restore) — create branches only via `git worktree add -b`.
   Note: shell permission matching is whole-command text, so compound
   commands smuggling a checkout (e.g. via `&&`) fall back to ask — stay in
   the worktree flow regardless.
4. Clean up after merge: `git worktree remove ../ezTopaz-<suffix>`,
   then `git worktree prune`. Verify with `git worktree list`.

## Notes

- `target/` and `node_modules/` are per-worktree; each needs its first build
  (`pnpm` store is shared so install is fast; Rust can share
  `CARGO_TARGET_DIR` but simultaneous builds block on the cargo lock).
- Branches, tags and remotes are shared across worktrees. Merge into `main`
  one at a time per `AGENTS.md` §1/§5.
