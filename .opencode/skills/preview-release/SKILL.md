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
5. Verify: `gh release view preview.XX --json assets --jq '.assets[].name'`

## Rules

- The PR-time and tag-time double build is intentional: published binaries
  must come from the tagged commit. Do not simplify.
- AI must never perform release operations (tag create/push, manual Release
  run, `gh release` publish) without explicit user instruction.
- Details: `AGENTS.md` §1/§2/§7.
