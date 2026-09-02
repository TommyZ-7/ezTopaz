# ezTopaz

TopazChat配信専用の軽量ストリーマー (MVP開発中)。
「起動 → 画面/音声選択 → 配信開始 → URLコピー」で完結します。

## 状態

- 詳細設計: `docs/design.md` (v0.2) / 要件定義: `docs/requirements.md` (v0.3.2)
- 実装済み: 設定管理・エンコーダprobe/引数生成・音声ミキサ・FramePacer・FFmpeg supervisor・named pipe (Unix/Windows)・UI一式・キャプチャバックエンド (Linux: Portal+PipeWire / Windows: WGC+WASAPI)
- 実装済み: F-ST-04 自動再接続 (3回・指数バックオフ・パイプ再生成)、配信前プレビュー (`start_preview`, 640x360@1fps)
- **キャプチャは未検証**: CI で両プラットフォームのコンパイル検証済み。実機での動作確認 (画面取得/アプリ別音声/配信E2E) が次の必須ステップ
- リリースビルド: CI `build` ジョブが pinned BtbN GPL ffmpeg を `src-tauri/resources/ffmpeg/` へ配置してバンドル (Windows: NSIS / Linux: deb)。ローカルでビルドする場合も同様に配置 (GPL表記必須, §11)。AppImage は同梱 ffmpeg への linuxdeploy patchelf 失敗のため保留 (§13.2 からの逸脱, CI コメント参照)

## 開発

```bash
pnpm install
cargo test --workspace   # 純粋ロジック (config/probe/args/mixer/pacer/progress)
pnpm test                # UIユーティリティ (vitest)
pnpm tauri dev           # デスクトップアプリ起動
pnpm tauri build         # リリースビルド (FFmpeg同梱は CI で pinned 公式GPLビルドを resources/ffmpeg/ へ配置)
```

要件: Linux は Wayland+PipeWire 必須 (X11非対応)、Windows は 10 2004+。

## 制約と注意

- TopazChat の上限: 映像 2000kbps / 音声 320kbps (超過で強制切断)。アプリ内でガードします
- StreamKey は公開情報 (視聴URLの一部) のため平文保存します
- TopazChat は個人運営・試験運用のため、映像配信は予告なく停止する可能性があります
- FFmpeg を libx264 (GPL) 同梱で配布する場合、対応ソースの提供義務があります

## ライセンス

- ezTopaz: MIT (`LICENSE`)
- 同梱 FFmpeg: LGPL 2.1+ / GPL (libx264 有効時) — `LICENSES/` 参照

## 支援

TopazChat 運営のよしたか氏: https://tyounanmoti.fanbox.cc/
