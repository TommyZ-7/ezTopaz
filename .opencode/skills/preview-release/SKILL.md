---
name: Preview Release
description: Ship an ezTopaz preview release (version bump, PR gate, tag, GitHub Release publish). Use when asked to release a preview version.
---

## Workflow

1. Bump version `0.1.0-previewXX` (semver forbids leading zeros, so `preview01`
   style, e.g. `preview.01` tag ↔ `0.1.0-preview01`):
   - `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`,
     `eztopaz-core/Cargo.toml`, `Cargo.lock`
   - `package.json` stays at `0.1.0`.
2. Branch → PR → `CI` + `Release` (bundle) green → merge into `main`.
3. Tag the updated `main` (only with explicit user instruction):
   `git tag preview.XX && git push origin preview.XX`
4. The tag-triggered `Release` workflow rebuilds deb/NSIS and publishes the
   GitHub Release. Monitor to completion (see `ci-watch` skill).
5. Verify assets and notes:
   `gh release view preview.XX --json assets --jq '.assets[].name'`
   `gh release view preview.XX --json body --jq .body`

## Release notes (must compare with the previous version)

- The workflow auto-generates the notes from `PREV...TAG`
  (`PREV` = one previous `preview.*` tag, full history via
  `checkout fetch-depth: 0`). Do not revert to a fixed template.
- Required sections:
  `### ✨ 追加 Added` (`feat:` commits),
  `### 🔧 改善 Improved` (all non-merge commits except `feat:`/`fix:`),
  `### 🐛 修正 Fixed` (`fix:` commits),
  plus a `Full Changelog: .../compare/PREV...TAG` link. Empty sections
  render as `- なし`.
- Template (first release omits the `(PREV からの変更)` suffix):

```md
## ezTopaz preview.XX (preview.YY からの変更)

### ✨ 追加 Added
- feat: ... (abc1234)

### 🔧 改善 Improved
- ... (def5678)

### 🐛 修正 Fixed
- fix: ... (789abcd)

---
**Full Changelog**: https://github.com/<owner>/<repo>/compare/preview.YY...preview.XX

MVP preview build. FFmpeg (BtbN GPL build) is bundled — ...
```

- Manual fix (e.g. wording polish) keeps the same format:
  `gh release edit preview.XX --notes-file /tmp/release-notes.md`

## Rules

- The PR-time and tag-time double build is intentional: published binaries
  must come from the tagged commit. Do not simplify.
- AI must never perform release operations (tag create/push, manual Release
  run, `gh release` publish) without explicit user instruction.
- Details: `AGENTS.md` §1/§2/§7.
