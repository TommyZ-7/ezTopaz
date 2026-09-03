#!/usr/bin/env bash
# Finalize a Tauri-built AppImage: inject ffmpeg + drop host-provided libs.
#
# 1. ffmpeg injection. linuxdeploy (invoked by `tauri build --bundles
#    appimage`) runs its bundled patchelf over every ELF in the AppDir,
#    including `usr/lib/<app>/resources/ffmpeg/ffmpeg`. The BtbN build cannot
#    be rewritten by that patchelf ("Failed to set rpath"), so the AppImage is
#    built WITHOUT the ffmpeg binary and it is added afterwards here:
#      AppImage --appimage-extract -> copy ffmpeg in -> appimagetool repack.
#    Two destinations (same inode via hardlink, stored once by mksquashfs):
#      usr/lib/<app>/resources/ffmpeg/ffmpeg  parity with the deb layout
#      usr/bin/ffmpeg                          resolves TODAY: ffmpeg_path()
#                                              only probes <exe-dir>/resources/
#                                              ffmpeg and <exe-dir>/ffmpeg, so
#                                              the resources/ copy alone would
#                                              be missed on Linux (same as deb).
#
# 2. Drop libs that MUST come from the host (verified on CachyOS/Mesa 26):
#    - libwayland-client.so.0: Ubuntu 24.04's wayland 1.22 makes host Mesa
#      fail EGL display creation ("Could not create default EGL display:
#      EGL_BAD_PARAMETER. Aborting..."). Removing only this file clears it;
#      libwayland-egl/-cursor/-server are harmless.
#    - libpipewire-0.3.so.0: Ubuntu's PipeWire 1.0.5 client cannot load SPA
#      support plugins against a newer host daemon ("can't make
#      support.system handle"), which breaks ALL PipeWire use in the bundle
#      (verified: host pw-dump fails the same way under the bundle's
#      LD_LIBRARY_PATH). Audio capture requires a working client, and
#      PipeWire is mandatory per requirements §6, so the host lib is used.
#    Safe because the AppImage already requires glibc 2.39+ (24.04 baseline),
#    whose distros all ship wayland >= 1.22 and PipeWire client libs.
#
# Usage: finalize-appimage.sh <appimage> <ffmpeg-bin> <appimagetool>
set -euo pipefail

# Absolute paths first: the script cd's into a mktemp workdir below,
# so relative caller paths would no longer resolve (exit 127).
APPIMAGE=$(readlink -f "$1")
FFMPEG_BIN=$(readlink -f "$2")
APPIMAGETOOL=$(readlink -f "$3")

for f in "$APPIMAGE" "$FFMPEG_BIN" "$APPIMAGETOOL"; do
  [ -e "$f" ] || { echo "finalize-appimage: not found: $f" >&2; exit 1; }
done
[ -x "$FFMPEG_BIN" ] || { echo "finalize-appimage: ffmpeg not executable: $FFMPEG_BIN" >&2; exit 1; }

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cd "$WORK"

# --appimage-extract needs no FUSE; drops ./squashfs-root in CWD.
"$APPIMAGE" --appimage-extract >/dev/null

RESDIR=$(find squashfs-root -type d -path '*resources/ffmpeg' | head -n1)
[ -n "$RESDIR" ] || { echo "finalize-appimage: resources/ffmpeg dir missing" >&2; exit 1; }
[ -d squashfs-root/usr/bin ] || { echo "finalize-appimage: usr/bin missing" >&2; exit 1; }

install -m755 "$FFMPEG_BIN" "$RESDIR/ffmpeg"
ln -f "$RESDIR/ffmpeg" squashfs-root/usr/bin/ffmpeg

# Host-provided libs (see header §2). rm -f: tolerate absence so the script
# keeps working if a future linuxdeploy stops bundling them.
rm -f squashfs-root/usr/lib/libwayland-client.so.0
rm -f squashfs-root/usr/lib/libpipewire-0.3.so.0

OUT="$WORK/repacked.AppImage"
ARCH=x86_64 "$APPIMAGETOOL" --appimage-extract-and-run squashfs-root "$OUT" >/dev/null
chmod +x "$OUT"

# sanity: executable ffmpeg at both spots, and no host-provided libs bundled
rm -rf squashfs-root
"$OUT" --appimage-extract >/dev/null
[ -x squashfs-root/usr/bin/ffmpeg ] || { echo "finalize-appimage: usr/bin/ffmpeg missing" >&2; exit 1; }
RESCHECK=$(find squashfs-root -path '*resources/ffmpeg/ffmpeg' | head -n1)
[ -n "$RESCHECK" ] && [ -x "$RESCHECK" ] || { echo "finalize-appimage: resources ffmpeg missing" >&2; exit 1; }
[ -z "$(find squashfs-root -name 'libwayland-client.so.0' -o -name 'libpipewire-0.3.so.0')" ] || { echo "finalize-appimage: host-provided lib still bundled" >&2; exit 1; }

mv "$OUT" "$APPIMAGE"
echo "finalize-appimage: OK -> $APPIMAGE"
