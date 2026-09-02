The pinned BtbN GPL ffmpeg binary is placed here before bundling:
- CI does this automatically (.github/workflows/ci.yml, `build` job).
- For a local release build, download the same pinned build (see the CI
  workflow for the exact tag/URL) and copy `bin/ffmpeg` (Windows: `ffmpeg.exe`)
  into this directory, or set EZTOPAZ_FFMPEG to an existing ffmpeg.

GPL note: the bundled ffmpeg is a GPL build (libx264) — the GPL notice must
ship with any distributed artifact (design §4.4 / §11).
