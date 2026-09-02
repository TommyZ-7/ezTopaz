# ezTopaz 要件定義書 v0.3.2

> 作成日: 2026-09-02 | 更新日: 2026-09-02 | 対象: 要件定義フェーズ | ステータス: Draft (v0.3.2 詳細設計整合) | リポジトリ: `ezTopaz`

---

## 1. はじめに

### 1.1 背景
VRChat内で画面共有を行う際、デファクトは `TopazChat` である。現状は `OBS Studio` で `rtmp://topaz.chat/live` へRTMP配信する運用だが、OBSはYouTube/Twitch等の汎用配信ソフトであり、TopazChat配信のたびに「サービス=カスタム」「サーバー/ストリームキー切替」「出力/映像設定の切替」が発生し運用負荷が高い。

### 1.2 目的
TopazChatへの映像・音声配信に特化し、「起動→画面/音声選択→配信開始→URLコピー」で完結する軽量配信ソフト `ezTopaz` (仮称) を提供する。OBSの汎用性を捨てる代わりにTopazChatの制約・推奨設定をデフォルト化し、設定迷子をゼロにする。

### 1.3 スコープ

- **含む**: 画面キャプチャ(全画面/ウィンドウ)、音声キャプチャ(システム/アプリ指定(複数選択・含める方式)/マイク)、エンコード・RTMP送信、プロファイル管理(低/中/高 + 1080p警告付)、URLコピー、Win/Linux同時対応(Wayland専用)、Ingest URL可変(MVPから対応)、エンコーダ手動選択、日英対応
- **含まない(今回)**: 録画機能(ローカル保存)、シーン合成(複数ソースのレイアウト)、仮想カメラ、クラウド機能、自動更新、テレメトリ

### 1.4 用語

| 用語 | 説明 |
|---|---|
| TopazChat | よしたか氏(@tyounanmoti)運営のVRChat向け低遅延配信サービス。個人利用無償。サーバはAWS等、費用は運営自費+FANBOXカンパ |
| TopazChat Player | ワールドに配置する受信用ギミック。BOOTH配布(https://booth.pm/ja/items/1752066) |
| TopazChat Streamer | 音声のみを簡易配信する公式アプリ(https://booth.pm/ja/items/1756789) |
| Ingest URL | OBS等が映像を送る先。デフォ `rtmp://topaz.chat/live` (本ソフトではMVPから編集可) |
| Playback URL | VRChat VideoPlayerに入力する視聴URL。PC: `rtspt://topaz.chat/live/{key}` / Quest: `rtsp://topaz.chat/live/{key}` (Player 3.0/3.3.1) |
| StreamKey | 配信を識別する英数字。衝突すると他人の配信と混線。URLの一部であり秘密鍵ではない |
| AAC 320kbps / 2Mbps | TopazChatの上限。超過で強制切断 |

---

## 2. TopazChat 詳細調査サマリ

> 出典: GitHub `TopazChat/TopazChat` README, BOOTH Player/Streamerページ, vrnavi.jp/ kohavrog 等複数ソースの突合。矛盾点は併記。

### 2.1 アーキテクチャ
```
[配信者PC: OBS/eZT] --RTMP--> [topaz.chat:1935/live/{key}] --RTSP/RTSPT--> [VRChat AVPro Player]
```
- Ingest: `RTMP` (`rtmp://topaz.chat/live`, FLVコンテナ, H.264 + AAC)
- Playback: `rtspt://` (TCP interleaved RTSP, PC低遅延) / `rtsp://` (Quest) / NeosVR等は `rtsp://`
- ingestsは `rtmp://` をVRChatに貼っても再生不可。必ず `rtspt(s)://` に変換が必要。

### 2.2 制約・推奨値 (公式+Booth引用)

| 項目 | 値 | 備考 |
|---|---|---|
| 映像ビットレート上限 | **2000 kbps 以下** (厳守) | 超過で強制切断。試験運用のため予告停止あり |
| 音声ビットレート上限 | **320 kbps 以下** (AAC Stereo推奨) | 320kbpsが最高。音楽用途は320維持推奨 |
| 遅延 | 東京でPC 0.3〜1秒, Quest 3〜5秒 | 距離・設定に依存。Global Sync/Resyncで補正 |
| フレームレート | **60fps 推奨** (公式「映像->60fps」) | 視聴側が30fps下回ると映像が大きく崩れる注意あり |
| フラグで遅延悪化 | 「輻輳管理でビットレート動的変更」「TCP pacing」ONで遅延増 | OFF推奨 |
| エンコーダNG例 | x264 `zerolatency` tune, NVENC `Low Latency` preset | 使用時VRChat側で灰色画面になることがある |
| NVENC推奨(Booth) | `NVENC / Max Performance / Profile High / Look-ahead OFF / Psycho Visual OFF / Max B-frames 0` | 低遅延最優先チューニング |
| 対応クライアント | 現状Windows VRChatのみ(公式)。Player 3.0+でQuest対応表記ありだが挙動差あり。Playerバージョンで `rtspt` vs `rtsp` を切替 | 要実機テスト |
| VRC設定 | 視聴者は「Allow Untrusted URLs」ON、Udonでは `VRCAVProVideoPlayer Use Low Latency` ON | |
| プレーヤ実装注意 | UdonSyncPlayerの定期的同期が音途切れを起こすため削除推奨 (公式) | ワールド作者向けだがFAQに記載 |

### 2.3 既知の不具合・注意
- 高負荷時に音途切れ→遅延蓄積。Resync/Global Syncで解消。
- VoiceMeeter Banana等の仮想デバイスで途切れやすい。
- ストリームキーは衝突しやすい汎用語(`test`, `music`等)は避ける。ランダム生成+固定ID保存を推奨。
- 映像配信は実験的機能。通信料高騰で停止リスクを明記する必要あり。

### 2.4 競合/類似
- **PebbleChat** (sawa-zen, HLS/480p/1Mbps/4秒遅延, OBS不要) : Topazより低負荷だが低画質・音声なし。棲み分け。
- 自前RTSPサーバ+RTMP: 上り帯域=視聴者数×ビットレートが必要で個人回線では非現実的。

---

## 3. ステークホルダー

| 区分 | 例 |
|---|---|
| 配信者 | VRChatイベント主催/演者/VJ/DJ、画面共有したい一般ユーザ |
| 視聴者 | VRChatワールド参加者(PC/Quest) ※本ソフトの直接ユーザではないがURL配布先 |
| ワールド作者 | TopazChat Player設置者、StreamKeyを発行/周知する人 |
| 運営 | 本ソフト開発・配布者 |

---

## 4. 開発言語・技術スタック選定

### 4.1 要求
- Windows / Linux 同時対応 (Wayland専用、X11非対応)
- モダン・軽量 (OBS 200MB+ / Electron 150MB+ は避ける)
- 画面/ウィンドウ列挙、音声デバイス列挙(アプリ別含む)、RTMP配信、エンコーダ制御 (HW accel) が可能
- FFmpegを内包またはサイドカーで扱える
- オフライン動作、MIT公開

### 4.2 候補比較

| 候補 | 構成 | バンドルサイズ | 長所 | 短所 | 判定 |
|---|---|---|---|---|---|
| **Tauri 2 + Rust + React** | Rustバックエンド + WebViewフロント + FFmpeg sidecar | 5-15MB (FFmpeg本体は別) | 最軽量、Rustで低レベルAPI直接叩ける、Win/Linuxネイティブ、セキュリティ良好、React人材豊富 | WebView依存(Win WebView2) | **◎ 推奨・採用** |
| Wails + Go + React | Go + WebView | 8-20MB | Goで並行処理簡潔 | 音声/画面の低レベルcrateがRustより弱い | ○ 次点 |
| Electron + TypeScript | Node + Chromium | 120-200MB | 開発容易 | 重い、要件の「軽量」に反する | × |
| Qt (C++/Python PySide) | Qt | 40-80MB | ネイティブUI | モダンさに欠ける、配布が重い | △ |
| Flutter | Dart | 20-40MB | クロスプラットフォーム | デスクトップは未成熟、FFmpeg連携が弱い | △ |

### 4.3 採用: **Tauri 2 + Rust (backend) + React + TypeScript (frontend) + FFmpeg**

**理由:**
1. 軽量: TauriはOS WebView利用でChromium同梱不要。FFmpeg同梱込みでもOBS(200MB+)/Electron(150MB+)に対し大幅に軽量(§6目標)
2. 既存資産: FFmpegがRTMP/H.264/AACのデファクト。再発明せずsidecarで呼ぶ
3. Native: 画面は `xdg-desktop-portal` + `pipewiregrab` (Wayland)、音声は `PipeWire` per-node / `WASAPI` loopback をRust crateで叩く
4. 同時リリース: TauriはWin/Linuxのビルドパイプラインが同一

**アーキテクチャ案:**
```
[React UI (ja/en)] <-> [Tauri IPC (Rust)] <-> [Capture Manager] <-> [FFmpeg sidecar]
                         |-> Portal+PipeWire (画面, Wayland専用)
                         |-> WASAPI (Win) / PipeWire (Linux) (音声, per-app)
                         |-> NVENC/QSV/AMF/x264 自動+手動選択
                         |-> Config (profiles.json + ingestUrl)
```

**FFmpeg内包方針:** FFmpegバイナリをインストーラに同梱し、sidecar crateは同梱バイナリを指定して起動する(初回DLは行わない。完全オフライン要件のため)。初回起動でHWエンコーダをprobe (`ffmpeg -encoders`) し自動選択。手動オーバーライドも設定画面で可能。画面/音声キャプチャはOS標準API(WGC/WASAPI/Portal+PipeWire)をRustで直接使用し、FFmpegはエンコード+RTMP専任(詳細は `docs/design.md`)。X11は非対応のためエラー表示。同梱要件は§4.4(v0.3.2注記参照)。

### 4.4 FFmpeg連携方式 (v0.3修正: キャプチャ経路を2系統に分離)

> **v0.3.2注記:** 詳細設計(`docs/design.md` v0.2)ではキャプチャを全てRust側に統一したため、**経路A(FFmpeg入力デバイス直)は不使用**。FFmpegはエンコード+RTMP専任となりキャプチャデバイスの有効化は不要(公式GPLビルドを同梱)。実装の基準は経路B。

> v0.2のCLI単体雛形を修正: ウィンドウキャプチャとアプリ別音声はFFmpegの入力デバイスに存在しない(`ddagrab`はデスクトップ全体のみ、プロセス単位ループバックはCLIに非露出、`pipewiresrc`はGStreamerの要素名でFFmpeg非該当)。キャプチャを下記2系統に分離する。

| 経路 | 対象 | キャプチャ主体 | FFmpegへの入力 |
|---|---|---|---|
| A: FFmpeg入力デバイス | 全画面(Win)、システム音声(Win)、全画面(Wayland) | FFmpeg | CLI完結 |
| B: Rustキャプチャ+パイプ | ウィンドウ(Win/Wayland)、アプリ別音声(Win/Linux)、マイク | Rust (WGC / WASAPI / Portal+PipeWire) | rawvideo / PCMパイプ |

> 経路Aの wasapi loopback(システム音声)と `pipewiregrab`(FFmpeg 8.0系で採用)は同梱ビルド構成依存。§12スパイクで確認し、不可なら該当経路はBに統一。

**経路A雛形 (全画面+システム音声, CLI完結):**

```bash
# Windows: 全画面 + システム音声 (60fps → GOP 2秒 = -g 120)
ffmpeg -f ddagrab -framerate 60 -i desktop \
  -f wasapi -i default \
  -c:v h264_nvenc -preset p1 -profile:v high -bf 0 -g 120 \
  -b:v 1500k -maxrate 1500k -bufsize 3000k \
  -c:a aac -b:a 192k -ac 2 -ar 48000 \
  -f flv rtmp://topaz.chat/live/{key}

# Linux Wayland: 全画面 + システム音声
ffmpeg -f pipewiregrab -framerate 60 -i portal \
  -f pulse -i @DEFAULT_MONITOR@ \
  -c:v libx264 -preset veryfast -profile:v high -bf 0 -g 120 -b:v 1500k \
  -c:a aac -b:a 192k -ac 2 -ar 48000 -f flv rtmp://topaz.chat/live/{key}
```

**経路B雛形 (Rustキャプチャ+パイプ):**
Rust側でキャプチャとミキシングまで行い、FFmpegへは1本の入力を渡す(複数stdinは不可のため、複数ソースはRustで合成)。

```bash
# 例: アプリ音声2本+マイクをRustで合成 (f32le 48kHz stereo) → パイプ
ffmpeg -f ddagrab -framerate 60 -i desktop \
  -f f32le -ar 48000 -ac 2 -i pipe:0 \
  -c:v h264_nvenc -preset p1 -profile:v high -bf 0 -g 120 \
  -b:v 1500k -maxrate 1500k -bufsize 3000k \
  -c:a aac -b:a 192k -ac 2 -ar 48000 \
  -f flv rtmp://topaz.chat/live/{key}

# 例: ウィンドウキャプチャ時は映像もRustから (rawvideo)
ffmpeg -f rawvideo -pix_fmt bgra -s 1280x720 -framerate 60 -i pipe:0 \
  -c:v h264_nvenc -preset p1 -profile:v high -bf 0 -g 120 \
  -b:v 1500k -maxrate 1500k -bufsize 3000k -f flv rtmp://topaz.chat/live/{key}
# 映像+音声の同時パイプ(named pipe/fd)方式は開発計画書のスパイクで確定
```

**同梱FFmpegビルド要件 (CIでカスタムビルド):**
- 有効化: `ddagrab`(Win), `wasapi`(loopback要検証), `pipewiregrab`(Linux Portal), `pulse`
- libx264フォールバック使用時は **GPLビルド必須** → GPLバイナリ配布のソース提供義務あり(§9参照)
- **初回DL禁止**: バイナリをインストーラに同梱し、sidecarは同梱パスを指定(完全オフライン要件)

---

## 5. 機能要件

### 5.1 画面キャプチャ (MUST)

| ID | 要件 | 詳細 | 優先度 | 備考(回答反映) |
|---|---|---|---|---|
| F-SC-01 | 全画面キャプチャ | マルチモニタ時はモニタ選択UIを表示。解像度/FPSはプロファイルに従う | Must | 同時リリース |
| F-SC-02 | ウィンドウキャプチャ | 起動中ウィンドウ一覧をサムネ付で列挙、選択 | Must | 全画面/単一で十分。WaylandではPortal経由が前提のため、コンポジタ側ピッカー委譲へ後退する可能性あり(§11リスク参照) |
| F-SC-03 | プレビュー | 配信前にローカルプレビュー(サムネ更新 1fps) | Must | |
| F-SC-04 | カーソル表示 | カーソル表示ON/OFF (初期ON) | Should | |
| F-SC-05 | Wayland対応 | Portal経由の画面共有許可ダイアログに対応。**X11は非対応**とし、X11環境ではエラー+Wayland切替案内 | Must | Waylandのみ |

### 5.2 音声キャプチャ (MUST, OBS同等 + α)

> 回答: 当初「Discord除外なども」→ 補足確認で「含めるのみで良い」に確定。除外(システム-Discord)は不要、含める方式で代替。

| ID | 要件 | 詳細 | 優先度 |
|---|---|---|---|
| F-AU-01 | システム音声 | OS全体の再生音(Desktop Audio)をキャプチャ。`F-AU-02a`と排他 | Must |
| F-AU-02a | アプリ指定(含める, 複数可) | **1つ以上のアプリ**の音声のみをキャプチャ (例: Chrome + Spotify)。システム音声と排他、アプリは複数選択可 | Must |
| F-AU-02b | アプリ除外 | システム音声から特定アプリを除外 | **Could (見送り)** |
| F-AU-03 | マイク入力 | 入力デバイス列挙・選択、**オンオフ(☑)・ミュート・ゲイン** | Must |
| F-AU-04 | ミキシング | 上記をミックスしてAAC 1本に。UIで各ソースのVU/ミュート/音量スライダー。複数アプリはRust側で合成しFFmpegへ1本のPCM(§4.4 経路B) | Must |
| F-AU-05 | デバイス永続化 | 選択デバイスをプロファイルに保存、未接続時は警告 | Must |
| F-AU-06 | サンプルレート | 48kHz固定、自動リサンプル | Must |

**実装方針:** Winは `WASAPI` でプロセス別セッション列挙(`IMMDeviceEnumerator + IAudioSessionManager2`)し、キャプチャはプロセスループバックAPI(Win10 2004+、§6互換参照)を使用。Linuxは `PipeWire` (Wayland前提)で node列挙+キャプチャターゲット指定。合成はRust側で実施しFFmpegへパイプ(§4.4 経路B)。`F-AU-02b` は今回見送り(含める方式で代替)。

### 5.3 エンコーダ・プロファイル (MUST)

| ID | 要件 | 詳細 | 優先度 | 備考 |
|---|---|---|---|---|
| F-EN-01 | プリセット切替 | **低/中/高 + 1080p** の4ボタンを常時表示 | Must | 1080pは警告付 |
| F-EN-02 | カスタムプロファイル | 追加/複製/削除/JSON編集可。保存先 `profiles.json` | Must | |
| F-EN-03 | 自動HW判定+手動選択 | 起動時に `h264_nvenc > h264_qsv > h264_amf(Win) > h264_vaapi(Linux) > h264_vulkan(実験) > libx264` で自動判定。`vulkan`は自動では`vaapi`優先、手動選択で`vulkan`可。設定画面で手動オーバーライド可 | Must | 手動もMVP |
| F-EN-04 | 上限ガード | 映像>2000k / 音声>320k は保存時にエラー/クランプ、UIで赤表示 | Must | |
| F-EN-05 | キーフレーム | GOP=2秒 (30fps→ -g 60, 60fps→ -g 120) 固定 | Must | |
| F-EN-06 | 詳細FFmpeg引数 | 上級者向けに追記テキスト欄 | Should | |

**デフォルトプロファイル (Topaz上限内で設計):**

| プロファイル | 解像度 | FPS | 映像kbps | 音声kbps | エンコーダ想定 | 用途 | UI表示 |
|---|---|---|---|---|---|---|---|
| 低画質 (Low) | 854x480 | 30 | 800 | 128 | libx264 veryfast | 低帯域/低スペック向け | [低] |
| 中画質 (Mid) **初期選択** | 1280x720 | 30 | 1500 | 192 | NVENC p1 / x264 veryfast | バランス推奨 | [中●] |
| 高画質 (High) | 1280x720 | 60 | 2000 | 320 | NVENC p1 / x264 veryfast | 最大品質 | [高] |
| 1080p (警告付) | 1920x1080 | 30 | 2000 | 320 | NVENC | 2000kではブロックノイズ有。警告表示 | [1080p ⚠] |

> 根拠: Topaz「2000以下・60fps・B-frames 0」+ OBSガイド 720p 1500-2500kをクランプ。1080pは含むがUIで「Topaz上限のため720p推奨」警告。

共通FFmpegオプション:
```
-pix_fmt yuv420p -profile:v high -bf 0 -g <fps*2> -maxrate <b:v> -bufsize <b:v*2>
-preset veryfast (x264) / -preset p1 (nvenc) / -preset veryfast (qsv)
-c:a aac -ac 2 -ar 48000 -f flv {ingestUrl}/{key}
```

### 5.4 配信制御 (MUST)

| ID | 要件 | 詳細 | 優先度 | 備考 |
|---|---|---|---|---|
| F-ST-01 | StreamKey入力 | 3-64文字、英数字/ハイフン/アンダースコア。空欄エラー、汎用キー警告 | Must | 保存して復元 |
| F-ST-02 | 配信開始/停止 | 大トグル1つ (灰:停止中 / 赤:配信中) | Must | |
| F-ST-03 | 状態表示 | 配信時間、推定ビットレート、ドロップ、VU | Must | |
| F-ST-04 | 自動再接続 | 切断時3回リトライ(指数バックオフ) | Should | |
| F-ST-05 | Ingest URL可変 | デフォ `rtmp://topaz.chat/live` を表示、**編集可**。VRCDN等他サービスにも対応 | Must | MVPから可変 |

### 5.5 ストリームURLコピー (MUST)

| ID | 要件 | 詳細 | 優先度 | 備考 |
|---|---|---|---|---|
| F-URL-01 | ワンクリックコピー | Key入力と同時に `rtspt://topaz.chat/live/{key}` 生成→コピー | Must | |
| F-URL-02 | 2種表示 | `rtspt://` (PC) と `rtsp://` (Quest) 各々コピー可、使い分けツールチップ | Must | 2種で十分 |
| F-URL-03 | QR/共有 | (将来) QR表示 | Could | 今回見送り |
| F-URL-04 | コピー通知 | 成功時トースト | Must | |

表示例:
```
Ingest: rtmp://topaz.chat/live  [編集可]  key: my-event-123
PC用:   rtspt://topaz.chat/live/my-event-123  [コピー]
Quest:  rtsp://topaz.chat/live/my-event-123   [コピー]
```

### 5.6 設定・永続化

| ID | 要件 | 詳細 |
|---|---|---|
| F-CF-01 | プロファイル保存 | `profiles.json` (Win `%APPDATA%/ezTopaz/`, Linux `~/.config/ezTopaz/`) |
| F-CF-02 | 前回値復元 | 起動時に前回の画面/音声/プロファイル/StreamKey/IngestURLを復元 | 
| F-CF-03 | エクスポート/インポート | プロファイルJSON入出力 |
| F-CF-04 | ログ | 配信ログを `logs/` に保存、UIから「ログを開く」 |
| F-CF-05 | 言語 | 日英切替。`ja`/`en` リソース、初期はOS言語自動選択 | Must (日英両対応) |

---

## 6. 非機能要件

| 区分 | 要件 | 目標値 | 備考(回答反映) |
|---|---|---|---|
| 性能 | CPU (720p30, x264 veryfast) | < 15% / NVENC時 < 5% | |
|  | メモリ | < 300MB | |
|  | 起動時間 | < 2秒 | |
|  | バンドルサイズ | Win < 100MB, Linux < 100MB (FFmpeg同梱込みのインストーラサイズ。FFmpegバイナリ単体で25-80MBのため実勢に合わせ設定。最適化はMVP後) | |
| 互換 | Windows | 10 2004+ / 11 (WASAPI, WGC。per-app音声(F-AU-02a)に必要なプロセスループバックAPIが2004+のため) | 同時リリース |
|  | Linux | **Ubuntu 22.04+/24.04, Arch (Waylandのみ、X11非対応, PipeWire必須)** | Wayland限定 |
| 信頼性 | 配信継続 | 瞬断で自動復帰、クラッシュ時FFmpeg確実kill | |
| 保守性 | ログ | FFmpeg stderr保存、UIに要約 | |
| セキュリティ | 権限 | 画面共有はPortal/WASAPI標準ダイアログ経由。Keyは平文保存(公開情報)明記 | |
| 配布 | インストーラ | Win: NSIS/msi, Linux: AppImage + deb/rpm (ArchはAUR想定)。**自動更新なし、手動DL** | 手動で良い |
|  | ライセンス | **MIT**でGitHub公開、BOOTH配布 | MITで公開 |
| 国際化 | 日英両対応 | MVPから `ja`/`en` 完全対応 | 日英両対応 |
| プライバシ | テレメトリ | **取得しない**、完全オフライン | 取得しない |
| 録画 | ローカル録画 | **なし** (配信専用) | 不要 |

---

## 7. 画面遷移・UI要件

### 7.1 画面構成

```
┌─────────────────────────────────────────┐
│ ヘッダ: ezTopaz | ●配信中 00:12 | 🌐ja/en | 設定 ⚙ |
├─────────────────────────────────────────┤
│ [画面] [音声] [出力]                    │
│ ■ 画面: ○全画面(モニタ1/2)              │
│         ○ウィンドウ ▼[Chrome]            │
│   [プレビュー 16:9]                     │
│ ■ 音声: ○システム全体  ▬○ VU            │
│         ○アプリ指定 ☑Chrome ☑Spotify ☑Discord VU │
│         ☑マイク ▼[USB Mic] VU  [オン/オフ] │
│ ■ 品質: [低] [中●] [高] [1080p⚠]       │
│         エンコーダ: [自動▼] 詳細▼       │
│ ■ 配信: Ingest [rtmp://topaz.chat/live ▼] [編集] │
│         key [my-key____]                │
│   PC  rtspt://...  [コピー]             │
│   Quest rtsp://... [コピー]             │
│   [ ■ 配信開始 ]  大ボタン              │
└─────────────────────────────────────────┘
```

- **原則:** OBSの「シーン/ソース」概念なし。チェックボックスとドロップダウンで完結。
- **配信開始ボタンは常時下部固定**。
- **VUメーター必須**。
- **エラー表示:** ビットレート超過、X11検出、デバイス未接続、Key空欄はインライン赤表示+配信開始無効化。
- **言語切替:** ヘッダの `ja/en` トグルで即時切替。

### 7.2 設定画面 (⚙)
- プロファイル一覧(低/中/高/1080p+カスタム) 編集・複製・削除
- エンコーダ: 自動結果表示 + 手動選択 (auto / x264 / nvenc / qsv / amf / vaapi / vulkan)
- 追加FFmpeg引数
- Ingest URL履歴
- ログ/バージョン/ライセンス(MIT)/支援リンク(Topaz FANBOX)

---

## 8. 外部IF・データ設計

### 8.1 外部IF
| 相手 | プロトコル | 方向 |
|---|---|---|
| topaz.chat:1935 (デフォ) / 任意Ingest | RTMP/FLV (H.264+AAC) | 送信のみ |
| OS画面API | WGC(Win) / Portal+PipeWire(Linux Wayland) | 取得 |
| OS音声API | WASAPI(Win) / PipeWire(Linux) | 取得 |
| クリップボード | OS | 書込 |

### 8.2 設定ファイル例

```json
// profiles.json
{
  "version": 2,
  "locale": "ja",
  "ingestUrl": "rtmp://topaz.chat/live",
  "activeProfile": "mid",
  "profiles": {
    "low":  { "name":"低画質", "w":854, "h":480, "fps":30, "v_kbps":800,  "a_kbps":128, "encoder":"auto" },
    "mid":  { "name":"中画質", "w":1280,"h":720, "fps":30, "v_kbps":1500, "a_kbps":192, "encoder":"auto" },
    "high": { "name":"高画質", "w":1280,"h":720, "fps":60, "v_kbps":2000, "a_kbps":320, "encoder":"auto" },
    "1080p":{ "name":"1080p", "w":1920,"h":1080,"fps":30, "v_kbps":2000, "a_kbps":320, "encoder":"auto", "warn":"Topaz上限2000kのため720p推奨" }
  },
  "lastStreamKey": "my-key",
  "lastSources": { "screen":"monitor:0", "includeApps":["Chrome","Spotify"], "mic":{"device":"default","enabled":true} },
  "encoderOverride": "auto"
}
```

---

## 9. 制約・前提

- TopazChatは個人運営・試験運用。**映像配信が予告なく停止するリスク**をアプリ内ヘルプとREADMEに明記。
- Ingest URLはMVPから可変だが、デフォはTopaz。VRCDN等への切替時はビットレート上限が異なるため、プロファイルの上限ガードをIngest別に将来拡張可能に。
- Linuxは**Wayland+PipeWire必須、X11非対応**。X11検出時は起動時にエラー表示しWayland移行を案内。
- ライセンス: 本ソフトは**MIT**で公開。libx264フォールバック使用時はGPLビルドのFFmpegを同梱するため、GPL該当ソースの提供義務をREADMEに明記し `LICENSES/` に同梱。
- テレメトリなし、自動更新なし。手動DLでの更新。

---

## 10. 受入基準 (抜粋)

| # | シナリオ | 期待結果 | MVP |
|---|---|---|---|
| AC-01 | 未設定で配信開始押下 | 各項目でエラー、配信開始されない | ● |
| AC-02 | 全画面/ウィンドウ切替→プレビュー切替 | 1秒以内に更新 | ● |
| AC-03 | マイクミュート→VU 0、配信に無音(システム音は載る) | ミキシング分離 | ● |
| AC-04 | Spotifyのみ「含める」→他アプリ音が載らない | Win WASAPI / Linux PipeWireで分離確認 | ● |
| AC-04c | Chrome+Spotifyを複数選択→2アプリの音がミックスされ他は載らない、マイクOFFで無音 | 複数アプリミックス確認 | ● |
| AC-04b | (見送り) 除外機能 | 今回は含める方式で代替 | — |
| AC-04d | マイクOFF→マイク音が載らずアプリ/システム音のみ、ONでミックス | マイクオンオフ確認 | ● |
| AC-05 | 「高」で配信→ ffprobeで2000k±10%, 320k, 60fps, yuv420p, GOP 2秒 | 上限遵守 | ● |
| AC-05b | 1080p選択→警告表示、配信は2000kで実行 | 警告付プロファイル | ● |
| AC-06 | Key `test-key-123` でコピー→ `rtspt://topaz.chat/live/test-key-123` | Questも同様 | ● |
| AC-06b | Ingest URLを `rtmp://custom.example/live` に変更→そちらへ配信 | 可変Ingest | ● |
| AC-07 | ネット切断→3回リトライ、復帰後継続 | | △ |
| AC-08 | Win10 2004+/11, Ubuntu 22.04 Wayland, Arch Waylandで起動・配信・視聴(Topaz Player)成功 | **X11では非対応エラー**を確認 | ● |
| AC-09 | 言語切替 ja/en→UIが即時切替、再起動後も保持 | 日英対応 | ● |
| AC-10 | エンコーダ手動でx264選択→ x264で配信、自動はHW優先 | 手動選択 | ● |

> MVP: ●=MVPリリース判定に使用 / △=F-ST-04がShouldのためMVP可否未決(§12) / —=見送り

---

## 11. リスク

| リスク | 影響 | 対策 |
|---|---|---|
| TopazChat仕様変更/停止 | 配信不可 | Ingest可変で代替(VRCDN等)へ切替可能に。ヘルプで代替案提示 |
| FFmpegバンドル肥大 | 配布サイズ増 | スリムビルドを同梱。初回DL方式はオフライン要件と矛盾のため不採用 |
| Linux X11で起動 | 取得失敗 | Wayland必須を明記、X11検出で明確エラー+案内 |
| Linux PipeWire未導入 | 音声分離不可 | 起動時チェック、未導入なら「システム+マイクのみ」にフォールバック+導入ガイド |
| HWエンコーダなし | 高負荷 | libx264フォールバック + 低画質自動提案 |
| Waylandでのウィンドウ一覧/サムネ列挙 (F-SC-02) | コンポジタ毎にAPI差異があり、アプリ側列挙が不可の環境がある | Portalネイティブのピッカーを第一動線とし、可能な環境のみアプリ側列挙を有効化。スパイクでGNOME/KDE/wlrootsを検証 |

---

## 12. 開発計画への申送り

- 次フェーズ「開発計画書」では: WBS/マイルストーン、技術スパイク(WASAPIプロセスループバック+Rust→FFmpeg PCMパイプ、PipeWire node列挙/ターゲットキャプチャ、Portal画面取得+`pipewiregrab`同梱ビルド、wasapi loopback対応確認)、FFmpeg同梱ビルドのCI構築、UIモック(日英)、テスト計画(Topaz Player実機検証、Ubuntu/Arch Wayland)を詳述。
- MVPスコープ確定: `F-SC-01/02/03/05, F-AU-01/02a/03/04, F-EN-01/02/03, F-ST-01/02/03/05, F-URL-01/02, F-CF-02/05` をMVP核とし、`F-SC-04, F-EN-06, F-ST-04` はShouldとしてMVPに含むか判断。
- 同時リリースのためCIでWin/Linux並行ビルド。X11は動作確認対象外とし、起動時の非対応エラー検出(AC-08)のみを確認対象とする。

---

## 13. 要確認事項 → 回答反映済み (v0.2で確定)

| Q | 質問 | 回答 | 反映先 |
|---|---|---|---|
| Q1 | OS優先度 | **同時リリース** | §6 互換、§12 |
| Q2 | Linux対象 | **Ubuntu + Arch、Waylandのみ、X11非対応** | §4.3, §5.1, §6, §9, §10 AC-08 |
| Q3 | 画面粒度 | **全画面/単一で十分** | §5.1 |
| Q4 | 音声ミキシング | **Discord除外なども → 補足で「含めるのみで良い」に確定** | §5.2 F-AU-02bはCouldへ |
| Q5 | per-app必須度 | Q4で包含 (含める方式のみ) | §5.2 |
| Q6 | プロファイル | **1080pも追加(警告付)** | §5.3 4プロファイル |
| Q7 | エンコーダ手動 | **手動も露出(MVP)** | §5.3 F-EN-03 |
| Q8 | Key保存 | **保存して復元** | §5.4, §5.6 |
| Q9 | URL表示 | **2種で十分** (rtspt/rtsp) | §5.5 |
| Q10 | ライセンス | **MITで公開** | §6, §9 |
| Q11 | 自動更新 | **手動で良い** (なし) | §6 |
| Q12 | テレメトリ | **取得しない** | §6, §9 |
| Q13 | 多言語 | **日英両対応** | §5.6, §6, §7 |
| Q14 | 録画 | **不要** | §1.3, §6 |
| Q15 | Ingest可変 | **MVPから可変** | §5.4 F-ST-05, §8.2 |

> 追加確認(解決): Q4補足「含めるのみで良い」で確定。除外UIは今回なし。システム全体かアプリ指定かの二択+マイク構成。

---

## 付録 A. 参考リンク

- TopazChat GitHub: https://github.com/TopazChat/TopazChat
- TopazChat Player (BOOTH): https://booth.pm/ja/items/1752066
- TopazChat Streamer (BOOTH): https://booth.pm/ja/items/1756789
- TopazChat Fanbox (支援): https://tyounanmoti.fanbox.cc/
- OBS Studio: https://obsproject.com/
- PebbleChat (類似軽量配信): https://sawa-zen.booth.pm/items/7919966
- vrchat + Topaz 遅延/設定解説: note.com/hairanndo, kohavrog.com/topazchat 等

## 付録 B. 変更履歴

| 版 | 日付 | 変更 |
|---|---|---|
| 0.1 | 2026-09-02 | 初版作成 |
| 0.2 | 2026-09-02 | Q1-Q15回答反映: 同時リリース/Wayland専用/除外機能/1080p警告/MIT/日英/Ingest可変/手動エンコーダ等を確定 |
| 0.2.1 | 2026-09-02 | Q4補足反映: 除外は見送り、含めるのみに修正 |
| 0.2.2 | 2026-09-02 | 複数アプリ対応: F-AU-02aを複数選択可に、マイクON/OFF明記 |
| 0.2.3 | 2026-09-02 | フロントをSvelte 5からReactに変更 |
| 0.2.4 | 2026-09-02 | エンコーダ判定順を `nvenc > qsv > amf > vaapi > vulkan > libx264` に更新 |
| 0.3 | 2026-09-02 | レビュー修正: Q4(除外見送り)の残骸清掃(§1.3/§7.1/§9/§11/§12/§13)、§4.4をキャプチャ2系統構成に修正+同梱FFmpegビルド要件/初回DL禁止/GOP修正、Win最低バージョンを2004+に更新、F-SC-02のWaylandリスク追加、ACにMVPゲート列追加 |
| 0.3.1 | 2026-09-02 | バンドルサイズ目標をFFmpeg同梱前提に大幅緩和(Win/Linux とも < 100MB)、§4.2/§4.3のサイズ記述を整合 |
| 0.3.2 | 2026-09-02 | 詳細設計v0.2と整合: §4.4に「経路A不使用/FFmpegはキャプチャデバイス不要(公式GPLビルド同梱)」の注記を追加 |

