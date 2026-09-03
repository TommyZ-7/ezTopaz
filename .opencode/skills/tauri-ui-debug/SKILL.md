---
name: Tauri UI Debug
description: Diagnose ezTopaz frontend failures (blank window, missing preview frames). Use when the Tauri window misbehaves on any platform.
---

## Symptom A: all-white window on every platform (dev and production)

Cause: zustand v5 `useStore` with an object-literal selector
(`useStore((s) => ({...}))`) returns a new snapshot each render, causing an
infinite re-render loop via `useSyncExternalStore`. No error boundary exists,
so the whole tree dies before first paint.

- Search: `useStore\(\(s\) => \(\{` and bare `useStore\(\)` under `src/`.
- Fix: split into individual selectors (`useStore((s) => s.field)`).
  Past cases: `ScreenSelector.tsx`, `AudioSelector.tsx`,
  `ProfileSelector.tsx`, `StreamControl.tsx`.
- Verify: `tsc --noEmit`; no object selectors remain.

## Symptom B: UI works, capture session runs (OS capture indicator visible), but no preview frames

Cause: missing `src-tauri/capabilities/default.json`. Without it, custom
`invoke()` calls pass (fail-open for local origin) so buttons work, but
`plugin:event|listen` is denied — `listen("stream://preview")` rejects
silently and no frame ever reaches the UI.

- Check: `src-tauri/capabilities/default.json` exists with
  `windows: ["main"]` and `core:event:default`.
- Adding it does not affect existing `invoke()` calls (no app ACL manifest).
- Verify: `cargo check -p eztopaz` regenerates
  `src-tauri/gen/schemas/capabilities.json` with the `default` entry.

## General

- Capture backend failures emit `stream://error`; the UI subscribes and
  toasts them (`src/App.tsx`). If an error is invisible, check the
  subscription first.
- WebView2/EGL warnings in WSL are software-rendering noise, usually unrelated.
