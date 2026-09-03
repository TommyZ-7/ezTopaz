# Arch Linux でのビルド・インストール

Arch系 (Arch / CachyOS / EndeavourOS / Manjaro) 向けの手順。Wayland + PipeWire が必須 (X11非対応、要件 §6)。

## インストール (AUR)

```bash
yay -S eztopaz-bin
```

公開前の場合は `packaging/aur/` の正本から手動ビルド:

```bash
cp -r packaging/aur /tmp/eztopaz-bin && cd /tmp/eztopaz-bin
makepkg -si
```

`eztopaz-bin` は GitHub Release の deb をリパックする。リリース毎の版数・ハッシュ更新は
`packaging/aur/PKGBUILD` 冒頭のコメントに従う (AURへの公開は手動)。

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

- **X11セッションで起動**: 非対応。Waylandセッションで起動し直す
  (`echo $XDG_SESSION_TYPE` で確認)
- **画面共有ピッカーが出ない**: 上表のバックエンドがDEに合っているか確認
- **PipeWire未導入**: システム音声+マイクのみにフォールバックする旨のトーストが出る。
  フル機能には `pipewire` (+ `pipewire-audio`) を導入
- **同梱FFmpeg**: BtbN linux64ビルドをそのまま同梱 (glibc後方互換でArchでも動作)
