# ezTopaz 引き継ぎ書 (v0.2 — preview.01 時点)

- 対象: TopazChat 配信専用ストリーマー MVP (Tauri 2 + Rust + React)
- 本書: `docs/impl-report.md` **v0.2**。preview.01 リリース時点の状態を引き継ぐ
- 前版: v0.1 (初回実装完了時点)。設計: `docs/design.md` (v0.2) / 要件: `docs/requirements.md` (v0.3.2)

## 1. 現状サマリ

- **CI 4ジョブ全グリーン** (run 33661442344 時点):
  - `linux`: core 51テスト + `capture-linux` コンパイル検証 + フロント (tsc strict/vite/vitest 4)
  - `windows`: core 51テスト + `capture-windows` コンパイル検証 (msvc)
  - `build (deb)`: **pinned BtbN GPL ffmpeg 同梱**の deb
  - `build (nsis)`: 同梱の NSIS インストーラ
- 実装済み: 設定管理 / エンコーダprobe・引数生成 / 音声ミキサ / FramePacer / supervisor / **named pipe (Unix/Windows 両方)** / キャプチャ (Linux: Portal+PipeWire, Windows: WGC+WASAPI) / F-ST-04 自動再接続 / 配信前プレビュー / UI一式
- **キャプチャは実機未検証**: 次の必須ステップは Wayland+PipeWire 実機での E2E (§5)

## 2. v0.1 からの変更点 (2026-09-02〜03 の CI 実装ラッシュ)

| 項目 | 状態 | 内容 |
|---|---|---|
| CI 初回実行→型エラー修正 | ✅ | capture-linux 8件 (ashpd 0.7 の start() フロー / ThreadLoop unsafe+Option / pw::keys::STREAM_ROLE 不在 → MEDIA_ROLE / From<VideoInfoRaw> 不在 → pod マクロ) + Windows 3件 |
| **Windows named pipe サーバ** (§4.1) | ✅ | `CreateNamedPipeW(OUTBOUND, byte)` + `ConnectNamedPipe`。`create`→ffmpeg spawn→`open_writer` の契約は unix FIFO と同一。**Windows の start_stream が動作可能に** |
| **F-ST-04 自動再接続** (§9) | ✅ | get_status が異常終了検知 → リトライスレッドがバックオフ (1/2/4s) → **パイプサーバ再生成 + キャプチャ + ffmpeg を再spawn** (3回上限、`retrying` でUI表示、stop で cancel) |
| **配信前プレビュー** (F-SC-03/§6.4) | ✅ | `start_preview`/`stop_preview`。キャプチャのみ (ffmpeg/パイプ無し) で 640x360@1fps PNG を `stream://preview` へ。UI にボタン + ja/en |
| **FFmpeg 同梱リリースビルド** (§13.2/§4.4) | ✅ | pinned BtbN GPL ビルドを `resources/ffmpeg/` へ配置し NSIS/deb を生成。`ffmpeg_path()` が同梱バイナリを解決 |
| **start_stream の実バグ修正** | ✅ | StreamProcess を state に保存していず get_status が必ずパニックする問題 (v0.1 時点) を修正 |
| AppImage | ⏸ 保留 | linuxdeploy が同梱 ffmpeg に patchelf rpath 設定失敗。復活手順は ci.yml コメント参照 (linuxdeploy ラッパー --exclude-files + librsvg2-dev) |

## 3. 検証状態

| 検証 | 方法 | 結果 |
|---|---|---|
| core テスト | `cargo test -p eztopaz-core` | 51/51, 警告0 |
| no-feature ビルド | `cargo check -p eztopaz` | OK |
| capture-linux | CI (`ubuntu-24.04` + libpipewire 1.0.5) | OK |
| capture-windows | CI msvc / ローカル `cargo check --target x86_64-pc-windows-msvc` | OK |
| フロント | `pnpm build` / `pnpm test` | OK (tsc strict / vitest 4) |
| **実機 E2E** | Wayland+PipeWire 実機 | **未実施 — 次の必須ステップ** |

### ローカル検証 Tips

- **capture-linux をローカルで**: システムの PipeWire が新しすぎると libspa 0.8 が壊れるため、
  Ubuntu noble の `libpipewire-0.3-dev`/`libspa-0.2-dev` を展開し `PKG_CONFIG_PATH` で指す。
  ただし bindgen 0.69 + 新しめの libclang (Arch の clang 22 等) では一部構造体が opaque 化する
  環境問題があり、その場合は CI 結果を正とする。
- **capture-windows をローカルで**: `rustup target add x86_64-pc-windows-msvc` →
  `cargo check --target x86_64-pc-windows-msvc --features capture-windows` (リンク不要なので check だけ可能)。
  `--tests` を付けると Windows 用テストコードも検査できる。
- node しか無い環境の pnpm: `curl -fsSL https://get.pnpm.io/install.sh | sh -` (ユーザ空間に導入)。

## 4. リリース (preview.01)

- バージョン: `0.1.0-preview01` (semver の前置ゼロ制限のため preview.01 ではなく preview01) / タグ: `preview.01` (タグ push で CI がビルド→GitHub Release 公開)
- 成果物: `ezTopaz_*_amd64.deb` (ffmpeg 同梱) / NSIS `.exe` (同梱)
- 同梱 ffmpeg: **pinned BtbN GPL ビルド** `autobuild-2026-09-02-13-13` (FFmpeg n8.1.2)
  - 更新手順: ci.yml の `FFMPEG_TAG`/`FFMPEG_DIR` を更新
  - **GPL (libx264) のため配布物に GPL 表記が必須** (設計 §11)
- AppImage は未提供 (§2 の保留参照)。Arch 等への配布は AppImage 復活後 or AUR (手動) かソースビルド

## 5. 残タスク (優先順)

1. **実機 E2E** (Wayland + PipeWire 実機, 要件 AC-04/05/06/08):
   起動 → Portalピッカー → 配信開始 → `ffprobe rtspt://topaz.chat/live/<key>` で確認。
   ffmpeg は BtbN ビルドを `EZTOPAZ_FFMPEG` 指定が最短。問題発生時は `logs/ezTopaz-*.log` を確認。
2. AppImage 復活 (linuxdeploy ラッパー --exclude-files + librsvg2-dev) または AUR パッケージ
3. F-AU-04: アプリ別ゲインUI (AudioSelector にスライダー/ミュート → `update_audio_mix`。バックエンドは実装済み)
4. 小口: `probe_encoders()` 結果キャッシュ (start_stream が全候補OKと仮定している `ponytail:` 箇所) / ログローテート / F-CF-03 export/import

## 6. ファイルマップ (v0.2)

```
.github/workflows/ci.yml   linux/windows テスト + build (deb/nsis) + タグで Release 公開
eztopaz-core/              純粋ロジック (どのOSでも cargo test 可)
  └ ffmpeg/
     ├─ pipes.rs           名前付きパイプ。unix: mkfifo / windows: CreateNamedPipeW サーバ (§4.1)
     ├─ start.rs           prepare (検証→パイプ生成→argv) + build_plan (純粋部)
     └─ supervisor.rs      spawn/progress/停止/retry_backoff。ffmpeg_path は同梱バイナリ解決
src-tauri/
  ├─ capture/linux.rs      Portal+PipeWire。start_screen は Portal fd を dup (消費しない) + 1fps プレビュー
  ├─ capture/windows/      WGC+WASAPI。screen.rs が stream://preview を 1fps 配信
  └─ ipc/commands.rs       AppState (stream/session/retrying/preview) / launch_pipeline /
                           start_stream / stop_stream / get_status(F-ST-04) / start_preview
src/                       React UI。StreamControl にプレビューボタン、Header に「再接続中 n/3」
src-tauri/resources/ffmpeg/ 同梱 ffmpeg 配置先 (CI が pinned ビルドを配置)
```

## 7. デビエーション (設計からの決定事項)

- v0.1 からの分に加えて:
  - **AppImage 保留 / Linux は deb のみ**: patchelf が同梱 ffmpeg に失敗するため (§4)。設計 §13.2 からの逸脱、復活手順は ci.yml 参照
  - **再接続時はパイプ+キャプチャごと再spawn** (impl-report v0.1 の「パイプは既存」から設計 §4.1 準拠に変更)。Windows パイプはインスタンスが1クライアントで消費されるため必須
  - **linux start_screen は Portal fd を消費しない**: preview → stream を再ピッカー無しで遷移させるため内部で dup
  - **プレビュー配信はキャプチャバックエンド自身が行う** (windows は既存実装、linux は preview スレッド追加)。RT スレッドを避けるため変換は別スレッド
- v0.1 からの既存分: workspace 分離 / キャプチャ全てRust側 / 音声パイプ f32le / vulkan 手動のみ / Portalピッカー第一動線 / Win 2004+

## 8. コミット履歴 (v0.1 引き継ぎ以降)

| コミット | 内容 |
|---|---|
| `9ddf313` | CI 初回失敗2件修正 (pnpm 10 / prepare を build_plan+パイプ生成に分離) |
| `38eb46f` | StartPlan Debug / ubuntu-22.04→24.04 (libspa 0.8 は新しめのヘッダ必須) |
| `3c9d0a7` | capture-linux 型エラー8件修正 + start_stream の StreamProcess 未保存バグ修正 |
| `37c560c` | 残り4件 (TARGET_OBJECT feature gate / sink 移動後使用 / 未使用引数) |
| `da88c1e` | 一括実装: Windowsパイプ / F-ST-04 / プレビュー / リリースCI |
| `7141af6`..`e3258fa` | CI 修正 (Windowsテスト新前提 / pwsh パス / artifact パス / verbose 診断) |
| `7c34974` | Linux を deb のみに (AppImage 保留を文書化) |
