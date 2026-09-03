---
name: E2E Stream Test
description: Manually verify ezTopaz end-to-end on a real machine (capture, streaming, output quality). Use when validating a preview build before or after release.
---

## Prerequisites

- Linux: Wayland + PipeWire (X11 unsupported). Windows: 10 2004+.
- An ffmpeg binary. Fastest: a BtbN build pointed to by `EZTOPAZ_FFMPEG`.

## Workflow

1. Launch the app → screen picker → start streaming.
2. Verify the output stream: `ffprobe rtspt://topaz.chat/live/<key>`
3. Acceptance (requirements AC-04/05/06/08, "high" profile):
   video 2000kbps ±10%, audio 320kbps, 60fps, yuv420p, GOP 2s.
4. On failure, inspect `~/.config/ezTopaz/logs/ezTopaz-YYYY-MM-DD.log`
   (ffmpeg stderr is streamed there).

## References

- `docs/impl-report.md` §5 (remaining tasks), `docs/design.md` §7.1 (logging),
  `docs/requirements.md` §10 (AC-04/05/06/08).
