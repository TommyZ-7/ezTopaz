# Arch Linux でのビルド・インストール

Arch系 (Arch / CachyOS / EndeavourOS / Manjaro) 向けの手順。Wayland + PipeWire が必須 (X11非対応、要件 §6)。

## インストール (AppImage)

GitHub Release の `.AppImage` をダウンロードして実行権限を付与:

```bash
chmod +x ezTopaz_*_amd64.AppImage
./ezTopaz_*_amd64.AppImage
```

- 実行には FUSE が必要 (`sudo pacman -S fuse2`)。FUSE が使えない環境では
  `./ezTopaz_*_amd64.AppImage --appimage-extract` で展開して `squashfs-root/usr/bin/eztopaz` を直接実行
- AppImage は ubuntu-24.04 でビルドしているため glibc 2.39+ が必要 (Arch系rollingでは問題なし)
- FFmpeg (BtbN GPLビルド) 同梱済み。linuxdeployの制約でビルド後に注入している (詳細は `packaging/appimage/inject-ffmpeg.sh`)

## インストール (AUR、任意)

`yay -S eztopaz-bin` (AURへの公開は手動。`packaging/aur/` の正本から手動ビルドも可):

```bash
cp -r packaging/aur /tmp/eztopaz-bin && cd /tmp/eztopaz-bin
makepkg -si
```

`eztopaz-bin` は GitHub Release の deb をリパックする。リリース毎の版数・ハッシュ更新は
`packaging/aur/PKGBUILD` 冒頭のコメントに従う。

## ソースビルド

```bash
# ビルド依存 (Tauri + PipeWire。Arch に -dev 分割は無い)
sudo pacman -S --needed base-devel git clang pkgconf \
  pipewire libpipewire webkit2gtk-4.1 gtk3
# Rust (rustup) と Node/pnpm は公式手順で用意

pnpm install
cargo test -p eztopaz-core
cargo check -p eztopaz --features capture-linux
pnpm build && pnpm test
pnpm tauri build --features capture-linux
```

pipewire-rs 0.10 が必要 (0.8 は clang 22 でビルド不可のため移行済み。詳細は #27)。

## 実行時依存

`eztopaz-bin` が依存として引くほか、画面共有には利用中のDEに対応する
xdg-desktop-portal バックエンドが必要:

| 環境 | バックエンド |
|---|---|
| GNOME | `xdg-desktop-portal-gnome` |
| KDE Plasma | `xdg-desktop-portal-kde` |
| Hyprland | `xdg-desktop-portal-hyprland` |
| Sway 等 wlroots 系 | `xdg-desktop-portal-wlr` |

音声のアプリ別キャプチャには PipeWire (および利用中なら `pipewire-audio`) が必要。

## トラブルシュート

- **`Could not create default EGL display: EGL_BAD_PARAMETER. Aborting...`**:
  preview.06 の AppImage で発生。ホストMesaと同梱wayland-clientの不整合が原因。
  preview.07 以降 (修正 #32) の AppImageを使用する
- **X11セッションで起動**: 非対応。Waylandセッションで起動し直す
  (`echo $XDG_SESSION_TYPE` で確認)
- **画面共有ピッカーが出ない**: 上表のバックエンドがDEに合っているか確認
- **PipeWire未導入**: システム音声+マイクのみにフォールバックする旨のトーストが出る。
  フル機能には `pipewire` (+ `pipewire-audio`) を導入
- **同梱FFmpeg**: BtbN linux64ビルドをそのまま同梱 (glibc後方互換でArchでも動作)
