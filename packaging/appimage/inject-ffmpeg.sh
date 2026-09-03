#!/usr/bin/env bash
# Inject the pinned BtbN ffmpeg binary into a Tauri-built AppImage.
#
# Background: linuxdeploy (invoked by `tauri build --bundles appimage`) runs
# its bundled patchelf over every ELF in the AppDir, including
# `usr/lib/<app>/resources/ffmpeg/ffmpeg`. The BtbN build cannot be rewritten
# by that patchelf ("Failed to set rpath"), so the AppImage is built WITHOUT
# the ffmpeg binary and it is added afterwards here:
#   AppImage --appimage-extract -> copy ffmpeg in -> appimagetool repack.
#
# Two destinations (same inode via hardlink, stored once by mksquashfs):
#   usr/lib/<app>/resources/ffmpeg/ffmpeg  parity with the deb layout
#   usr/bin/ffmpeg                          resolves TODAY: ffmpeg_path() only
#                                           probes <exe-dir>/resources/ffmpeg
#                                           and <exe-dir>/ffmpeg, so the
#                                           resources/ copy alone would be
#                                           missed on Linux (same as deb).
#
# Usage: inject-ffmpeg.sh <appimage> <ffmpeg-bin> <appimagetool>
set -euo pipefail

APPIMAGE="$1"
FFMPEG_BIN="$2"
APPIMAGETOOL="$3"

for f in "$APPIMAGE" "$FFMPEG_BIN" "$APPIMAGETOOL"; do
  [ -e "$f" ] || { echo "inject-ffmpeg: not found: $f" >&2; exit 1; }
done
[ -x "$FFMPEG_BIN" ] || { echo "inject-ffmpeg: ffmpeg not executable: $FFMPEG_BIN" >&2; exit 1; }

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cd "$WORK"

# --appimage-extract needs no FUSE; drops ./squashfs-root in CWD.
"$APPIMAGE" --appimage-extract >/dev/null

RESDIR=$(find squashfs-root -type d -path '*resources/ffmpeg' | head -n1)
[ -n "$RESDIR" ] || { echo "inject-ffmpeg: resources/ffmpeg dir missing" >&2; exit 1; }
[ -d squashfs-root/usr/bin ] || { echo "inject-ffmpeg: usr/bin missing" >&2; exit 1; }

install -m755 "$FFMPEG_BIN" "$RESDIR/ffmpeg"
ln -f "$RESDIR/ffmpeg" squashfs-root/usr/bin/ffmpeg

OUT="$WORK/repacked.AppImage"
ARCH=x86_64 "$APPIMAGETOOL" --appimage-extract-and-run squashfs-root "$OUT" >/dev/null
chmod +x "$OUT"

# sanity: the final artifact must contain an executable ffmpeg at both spots
rm -rf squashfs-root
"$OUT" --appimage-extract >/dev/null
[ -x squashfs-root/usr/bin/ffmpeg ] || { echo "inject-ffmpeg: usr/bin/ffmpeg missing" >&2; exit 1; }
RESCHECK=$(find squashfs-root -path '*resources/ffmpeg/ffmpeg' | head -n1)
[ -n "$RESCHECK" ] && [ -x "$RESCHECK" ] || { echo "inject-ffmpeg: resources ffmpeg missing" >&2; exit 1; }

mv "$OUT" "$APPIMAGE"
echo "inject-ffmpeg: OK -> $APPIMAGE"
