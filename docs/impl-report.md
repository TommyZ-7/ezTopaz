# ezTopaz 実装完了レポート(引き継ぎ書)

> 作成日: 2026-09-02 | 対象コミット: `cd57709`(スキャフォールド)〜 `c44356b`(最新) | 対応: 要件定義書 v0.3.2 / 詳細設計書 v0.2

---

## 1. サマリ

要件定義書・詳細設計書に基づく MVP の実装を完了した。**純粋ロジックは全て実装+テスト済み(50件)**、キャプチャバックエンド(Linux/Windows)は**実装済みだが実機未検証**(コンパイル検証のみ)。動く状態までの残りは「CI初回実行→型エラー修正」「実機E2E」「Windows named pipe」「F-ST-04再接続」「配信前プレビュー」の5点(§7)。

## 2. 実装済み機能(設計書との対応)

| 設計 § | 機能 | 実装 | 状態 |
|---|---|---|---|
| §3.1.3 | FramePacer / 解像度正規化 | `eztopaz-core/src/video/mod.rs`(pacer)+`video/sink.rs`(scale_bgra) | ✅ テスト済み |
| §3.2.3 | 音声ミキサ(f32合成/VU) | `eztopaz-core/src/audio/mod.rs` | ✅ テスト済み |
| §4.1–4.3 | VideoSink/AudioSink(パイプ書込) | `video/sink.rs`, `audio/sink.rs` | ✅ テスト済み |
| §4.3 | FFmpeg引数生成(エンコーダ別preset/RC) | `ffmpeg/args.rs` | ✅ テスト済み |
| §8.1 | エンコーダprobe(1フレーム実機テスト) | `ffmpeg/probe.rs` | ✅ パステスト済み(実行時テストは実機) |
| §4.1/§9 | supervisor(spawn/kill/progress解析/ログ) | `ffmpeg/supervisor.rs`, `progress.rs` | ✅ テスト済み(RUN_E2E以外) |
| §4.2 | named pipe | `ffmpeg/pipes.rs` | ✅ unix / ❌ Windows未実装 |
| §5 | Tauri IPC 14コマンド | `src-tauri/src/ipc/commands.rs` | ✅ |
| §6 | React UI(タブ/VU/プレビュー/設定/i18n) | `src/` | ✅ ビルド通過 |
| §7 | profiles.json(atomic保存, 上限ガード) | `eztopaz-core/src/config.rs` | ✅ テスト済み |
| §3.1/3.2 | Linuxキャプチャ(Portal+PipeWire) | `src-tauri/src/capture/linux.rs` | ⚠️ コンパイル未検証(CI待ち) |
| §3.1/3.2 | Windowsキャプチャ(WGC+WASAPI) | `src-tauri/src/capture/windows/` | ⚠️ msvcターゲットでコンパイル検証済み、実行未検証 |

## 3. 検証状態

| 項目 | 結果 |
|---|---|
| `cargo test --workspace` | **50/50 OK**(警告0) |
| `pnpm build`(tsc strict + vite) | OK |
| `pnpm test`(vitest) | 4/4 OK |
| `cargo check -p eztopaz --features capture-windows --target x86_64-pc-windows-msvc` | OK |
| `cargo check -p eztopaz --features capture-linux` | **ローカル実行不可**(sudo無しでlibpipewire-devを導入できないため)→ CI で実施 |
| 実機E2E(AC-01〜10のうち手動項目) | **未実施** |

**開発環境の制約(引継ぎ先であれば解消可能)**: 実装に使用したLinux機は sudo 不可・`libpipewire-dev` 未導入・`ffmpeg` コマンド無し。このため Linux capture のローカルコンパイルと E2E ができない。

## 4. 既知の未解決事項・リスク(最重要)

1. **Linux capture は型エラーの可能性が高い** — `pipewire-rs 0.8` / `ashpd 0.7` のAPIは実物ソース照合で書いたが、コンパイラは通していない。CI初回実行でエラーが出る前提で修正に1〜2時間を見込むこと。照合に使ったパス: `pipewire::thread_loop::ThreadLoop`(start/stop/wait/downgrade)、`Stream::connect(direction, id: Option<u32>, flags, params)`、`VideoInfoRaw::parse/size()`、`ashpd Screencast::open_pipe_wire_remote → RawFd`。
2. **Windows named pipe サーバ未実装**(設計§4.1スパイク)— `pipes.rs` の windows 分岐は `NotImplemented`。`start_stream` は Windows で明確なエラーを返す(WGC/WASAPIキャプチャ本体は実装済みで接続待ち)。
3. **F-ST-04 自動再接続が未接続** — `supervisor.rs` に `MAX_RETRIES` / `retry_backoff_ms()` はあるが、`commands.rs` の再接続ループは未実装。`StreamStatus.retrying` は常に `None`。
4. **配信前プレビューが未達**(要件 F-SC-03 は「配信前」)— 現状キャプチャは配信中のみ起動するため、プレビューは配信中にしか出ない。配信前にキャプチャのみ起動するプレビューモードの追加が必要。
5. **アプリ別ゲインUIが暫定** — `AudioSelector` の `pushMix` が全アプリに `mic.gain` を流す。ソース別スライダー(F-AU-04)のUIが必要。
6. **FFmpeg同梱が未整備** — pinned BtbN GPLビルドの `resources/ffmpeg/` 配置と CI ステップ未作成(設計§4.4)。無い場合は `EZTOPAZ_FFMPEG` 環境変数 or PATH の ffmpeg を使用。
7. その他小口: ログローテート未実装(メモリバッファ上限のみ)、F-CF-03 export/import 未実装(要件で優先度未割当)、プロファイル変更は配信中不可(パイプ形式固定=設計通り)、`AudioInfoRaw::parse` 後のレートが48k以外の場合のリサンプルは `resample_stereo` で対応済みだが PipeWire 側のネゴシエーション実機確認が必要。

## 5. ファイルマップ

```
docs/                    requirements.md (v0.3.2) / design.md (v0.2) / この文書
.github/workflows/ci.yml CI: linux(capture-linux check) / windows(check) / core tests / frontend

Cargo.toml               workspace (eztopaz-core + src-tauri)
eztopaz-core/            純粋ロジック。どのOSでも cargo test 可能
  ├─ config.rs           profiles.json, 上限ガード(2000k/320k), StreamKey検証, デフォルト4プロファイル
  ├─ error.rs            共有エラー型
  ├─ ipc_types.rs        Tauri共有型 (camelCase JSON)
  ├─ audio/mod.rs        Mixer(f32加算/clamp/ゲイン/VU), resample_stereo
  ├─ audio/sink.rs       AudioSink: ソース別キュー→ミックス→f32le書込
  ├─ video/mod.rs        FramePacer(最終フレームfps複製)
  ├─ video/sink.rs       VideoSink: 新着優先+fps送出, scale_bgra(レターボックス), bgra_to_rgba
  └─ ffmpeg/
     ├─ probe.rs         -encodersパース, 1フレームテストコマンド, 自動選択順
     ├─ args.rs          build_ffmpeg_args(エンコーダ別preset/RC, GOP=2s, 上限ガード)
     ├─ start.rs         StartPlan(検証→パイプ生成→argv)純粋関数+テスト
     ├─ supervisor.rs    StreamProcess(spawn/stop/Drop-kill, -progress解析, stderr→logs/)
     └─ pipes.rs         unix mkfifo(idempotent) / windows=NotImplemented

src-tauri/
  ├─ tauri.conf.json     CSP, 960x720, bundle icon
  ├─ src/main.rs         handler登録
  ├─ src/ipc/commands.rs 14コマンド + AppState(start/stop/vu/mix まで含むストリーム配線)
  └─ src/capture/
     ├─ mod.rs           CaptureError, feature gating
     ├─ linux.rs         portal_picker(async) / start_screen / start_audio / list_audio_devices
     └─ windows/         screen.rs(WGC) / audio.rs(WASAPI) / enumerate.rs / mod.rs

src/                     React (Vite+TS+Tailwind4+zustand+i18next)
  ├─ store.ts            全状態+ポーリング(status 1s / vu 100ms)
  ├─ lib/{api,types,urls}.ts   IPCラッパ / 型 / URL生成+キー検証(vitest 4件)
  ├─ components/         Header, ScreenSelector, AudioSelector, ProfileSelector, StreamControl, SettingsModal
  └─ locales/{ja,en}.json
```

## 6. コマンド

```bash
pnpm install
cargo test --workspace            # ロジック 50件
pnpm build && pnpm test           # フロント
cargo check -p eztopaz --features capture-windows --target x86_64-pc-windows-msvc  # Windows側
cargo check -p eztopaz --features capture-linux                    # Linux側(CI or libpipewire-dev環境)
pnpm tauri dev                    # 起動(LinuxはWaylandセッション必須)
pnpm tauri build                  # リリース(FFmpeg同梱は未整備、§4 の6参照)
EZTOPAZ_FFMPEG=/path/to/ffmpeg    # 同梱無し時にPATH以外を指定
```

## 7. 引き継ぎタスク(推奨順)

1. **push して CI を実行** → `capture-linux` の型エラー修正(§4 の1)。`cargo fetch` 済みのソースは `~/.cargo/registry/src/*/pipewire-0.8.0` 等で参照可能。
2. **Linux 実機 E2E**(Wayland + PipeWire + ffmpeg 必須): 起動 → Portalピッカー → 配信開始 → `ffprobe rtspt://...` で AC-04/05/06/08 を確認。ffmpeg は BtbN GPL ビルドを `EZTOPAZ_FFMPEG` で指定するのが最短。
3. **F-ST-04 再接続**: `commands.rs` `get_status` 内で ffmpeg 異常終了を検知 → `retry_backoff_ms(retry)` 待ち → 同一 `StartPlan` で `StreamProcess::spawn` 再試行(パイプは既存、FFmpegのみ再spawn。上限3回、`StreamStatus.retrying` をUIへ)。
4. **配信前プレビュー**(F-SC-03): `start_preview` コマンドを追加(キャプチャのみ起動、FFmpeg/パイプ無し、PreviewFrameイベント配信)。停止は stop と共通化。
5. **Windows named pipe サーバ**(設計§4.1): `windows` crate で `CreateNamedPipeW(PIPE_ACCESS_INBOUND)` + `ConnectNamedPipe` → `pipes.rs` の windows 分岐を実装。実装すれば WGC/WASAPI がそのまま繋がる。
6. **FFmpeg同梱 + 配布物**: CI で pinned BtbN GPL ビルドを DL → `resources/ffmpeg/` へ配置 → `pnpm tauri build` で NSIS/AppImage を出力。GPL表記(§11)を確認。
7. **アプリ別ゲインUI**(F-AU-04): AudioSelector にソース別スライダー/ミュート → `update_audio_mix`。
8. 小口: ログローテート、F-CF-03 export/import、`probe_encoders()` 結果のキャッシュ(start_stream が毎回全候補OKと仮定している `ponytail:` コメント箇所)。

## 8. 設計からの意図的なデビエーション(決定事項)

- **eztopaz-core ワークスペース分離**(設計§2.3を更新済み): プラットフォーム非依存ロジックをどのOSでもテスト可能にするため。
- **キャプチャは全てRust側**(要件§4.4 経路A不使用、v0.3.2注記済み): FFmpeg はエンコード+RTMP専任。同梱FFmpegは公式GPLビルドで足りる(セルフビルド不要)。
- **音声パイプは f32le**(s16leではなく): WASAPI/PipeWireネイティブがf32のため変換往復を削減。
- **vulkan は手動選択のみ**(自動順から除外)、probeは `-encoders` 解析+1フレーム書き出しテスト(コンパイル済み≠動作するため)。
- **Portalピッカー第一動線**: アプリ側のウィンドウ列挙は持たない(設計§3.1.2)。
- **Windows最低バージョン 2004+**(プロセスループバックAPI)、バンドルサイズ目標 <100MB(v0.3.1緩和)。

## 9. コミット履歴(実装順)

| コミット | 内容 |
|---|---|
| `cd57709` | スキャフォールド(workspace, Tauri 2, Vite+Tailwind+zustand+i18next) |
| `34b6af2` | config モジュール + テスト6件 |
| `987f240` | ffmpeg probe/args + テスト8件 |
| `263f22d` → `0b3dd40` | mixer/FramePacer/共有型 + テスト(確定的修正) |
| `2c81b6d` | supervisor + named pipe + IPC 14コマンド |
| `d7c4520` | ログファイル/LICENSE/README |
| `84e24a7` | React UI 一式 |
| `53bb89a` → `514f542` | VideoSink/AudioSink + スケーラ/リサンプラ |
| `d1810d7` | Windows capture(WGC+WASAPI, msvc検証) |
| `c42a266` | Linux capture(Portal+PipeWire) + CI |
| `e7b0e82` | UI配線(VU/preview/ライブミックス) |
| `c44356b` | README状態更新 |
