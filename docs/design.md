# ezTopaz 詳細設計書 v0.2

> 作成日: 2026-09-02 | 要件定義書: `docs/requirements.md:1` v0.3.2 対応 | ステータス: Draft (v0.2 レビュー修正反映)

---

## 1. 設計方針

- **要件定義書の責務分離:** 要件定義書(`requirements.md`)がWHAT、本書がHOWを担う。`§4.4` のCLI例の不確実性(0.3評価)を解消し、検証済みアーキテクチャを固定
- **YAGNI徹底:** 録画/シーン合成/自動更新/テレメトリは作らない。画面は全画面/単一ウィンドウ、音声は「システム or アプリ複数 + マイク」で割り切る
- **Tauri 2 + Rust + React:** 軽量(§6 <100MB緩和だが実測 <50MBを目指す)、Wayland専用、オフライン完結
- **FFmpegは同梱だがデバイス名は仮定しない:** `wasapi/pipewiregrab` の存在を仮定せず、全音声とウィンドウ映像はRustで取得しFFmpegへパイプで渡す。FFmpegはエンコード+RTMP送信に専念(**キャプチャデバイス不要のため公式GPL事前ビルドの同梱で足りる**。§4.4)
- **フレーム供給はRustが司る:** WGC/Portalはコンテンツ変化時のみフレームを出す。Rust側で「最終フレームのfps複製送出」と「プロファイル解像度への正規化」を行い、パイプのフォーマットを起動中不変に保つ(§3.1.3)

---

## 2. システムアーキテクチャ

### 2.1 全体構成

```
┌─────────────────────────────────────────────────────────┐
│  React UI (Vite + Tailwind, ja/en)                     │
│   Header / ScreenSelector / AudioSelector /            │
│   ProfileSelector / StreamControl / LogView            │
└──────────────────────┬──────────────────────────────────┘
                       │ Tauri IPC (invoke/event)
┌──────────────────────▼──────────────────────────────────┐
│  Rust Backend (Tauri 2)                                  │
│  ┌──────────────┐  ┌────────────┐  ┌─────────────────┐  │
│  │CaptureManager│──│AudioMixer  │──│ FFmpegSupervisor│  │
│  │ Screen/Audio │  │(Rust合成)  │  │  (sidecar)      │  │
│  └──────┬───────┘  └─────┬──────┘  └────────┬────────┘  │
│         │                │                  │            │
│   ┌─────▼─────┐    ┌─────▼─────┐    ┌──────▼──────┐     │
│   │WGC/Portal │    │WASAPI/    │    │  RTMP FLV   │     │
│   │PipeWire   │    │PipeWire   │    │ rtmp://...  │     │
│   └───────────┘    └───────────┘    └─────────────┘     │
│  ┌───────────────────────────────────────┐              │
│  │ ConfigManager (profiles.json)         │              │
│  └───────────────────────────────────────┘              │
└─────────────────────────────────────────────────────────┘
```

### 2.2 技術スタック確定

| 層 | 技術 | バージョン/備考 |
|---|---|---|
| Backend | Tauri 2 + Rust | `tauri 2.1`, `tokio` async |
| Frontend | React + TypeScript + Vite + Tailwind | `react 18`, `zustand` 状態管理 |
| 画面(Way) | `windows-rs 0.52` (WGC), `ashpd 0.7` + `pipewire 0.7` | Portal経由 |
| 画面(Linux) | `ashpd` + `pipewire` | Wayland専用, X11は起動時エラー |
| 音声(Win) | `wasapi` crate + `ActivateAudioInterfaceAsync` (Process Loopback API, 2004+) | per-appは `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK` |
| 音声(Linux) | `pipewire` crate | node列挙+キャプチャターゲット指定(pulse層は不使用) |
| エンコード | FFmpeg sidecar (カスタムビルド同梱) | `ffmpeg-sidecar 2.x` |
| 設定 | `serde_json` | `~/.config/ezTopaz/profiles.json` |
| i18n | `i18next` (React) | ja/en |

### 2.3 構成 (Cargo workspace)

```
ezTopaz/
 ├─ Cargo.toml          # workspace (eztopaz-core + src-tauri)
 ├─ eztopaz-core/       # 純粋ロジック (config / ffmpeg probe・args / mixer / pacer / supervisor / 共有型)
 │                      #   プラットフォーム非依存。cargo test がどのOSでも通る
 ├─ src-tauri/          # Tauri glue + プラットフォームキャプチャ
 │   ├─ src/main.rs
 │   ├─ src/ipc/        # commands.rs
 │   ├─ src/capture/    # windows.rs (feature: capture-windows) / linux.rs (feature: capture-linux)
 │   └─ icons/
 ├─ src/                # React frontend
 └─ resources/ffmpeg/   # 同梱バイナリ (Win: ffmpeg.exe, Linux: ffmpeg) ※pinned公式GPLビルド
```

---

## 3. キャプチャ設計

### 3.1 画面キャプチャ

#### 3.1.1 Windows

| 対象 | 実装 | 備考 |
|---|---|---|
| 全画面 | `WGC: GraphicsCaptureItem::CreateForMonitor` | マルチモニタは `HMONITOR` 列挙、`output_idx` 選択 |
| ウィンドウ | `WGC: CreateForWindow(HWND)` | サムネは `D3D11` テクスチャから `BGRA` 取得。最小化ウィンドウは警告。`ddagrab` は使わない(全画面でもWGCに統一し分岐を減らす) |
| カーソル | `GraphicsCaptureSession::IncludeCursor` | ON/OFF切替 |
| プレビュー | Rust側で 640x360 に縮小 → 1fps に間引き `base64 PNG` を `event` でReactへ送信 | FFmpeg経由しない。フルサイズのまま送ると数MB/sのIPCになるため必ず縮小 |

> WGC/Portalはいずれもコンテンツ変化時のみフレームを供給する。そのままrawvideoパイプに繋ぐと静的画面でエンコードが止まるため、§3.1.3のFramePacerを必ず通す。

**クレート:** `windows 0.52` (`Windows.Graphics.Capture`, `Windows.Graphics.DirectX.Direct3D11`), `windows-capture` 0.1 (ラッパ) を検討だが、自前実装でYAGNI

#### 3.1.2 Linux (Wayland専用)

| 対象 | 実装 |
|---|---|
| 全画面/ウィンドウ | `ashpd::desktop::screencast::Screencast` でPortal呼び出し → ユーザはOSのピッカーで選択 → 返却された `PipeWire` fd を `pipewire` crateで `Stream` 接続 → `SPA_VIDEO_Format` から `BGRA` 取得 |

- **F-SC-02 リスク対応:** Portalピッカーを第一動線とし、アプリ側でウィンドウ一覧を列挙する方式は持たない(コンポジタ差異が大きいため)。つまり「全画面か、Portalが提示するウィンドウ」から選ぶ
- **X11検出:** 起動時に `XDG_SESSION_TYPE` と `WAYLAND_DISPLAY` を確認。`x11` なら `Error::X11NotSupported` を返し、Reactでモーダル表示

#### 3.1.3 フレーム供給ポリシー (Win/Linux共通)

WGC/Portalはフレームをコンテンツ変化時にのみ供給し、かつソースサイズは起動中に変わりうる(ウィンドウリサイズ等)。rawvideoパイプの整合を保つため、キャプチャとパイプ送出の間に**FramePacer**を置く。

```
[WGC/Portal] --変化時のみ--> FramePacer --正規化--> video_tx (named pipe)
                                 ├─ 最終フレームを保持し、fpsで複製送出(静的画面でもストリームが止まらない)
                                 └─ 全フレームをプロファイルの w×h にスケール+レターボックス(リサイズ耐性、パイプ形式を不変に)
```

- スケールはGPU(D3D11 VideoProcessor等)を第一候補、不可ならCPU。方式はスパイクで確定(§14)
- プロファイル変更は配信中不可(パイプ形式が不変のため)。プロファイル切替は配信前のみ(`F-EN-01`)

### 3.2 音声キャプチャ

#### 3.2.1 Windows (WASAPI)

- **列挙:** `IMMDeviceEnumerator::EnumAudioEndpoints(eRender/eCapture)` でデバイス列挙、`IAudioSessionManager2::GetSessionEnumerator` で per-app セッション列挙。`ISessionControl2::GetProcessId` → `QueryFullProcessImageName` で `Chrome.exe` 等の名前取得
- **キャプチャ:**
  - システム全体: `IAudioClient::Initialize(SHARED, LOOPBACK)` で `default` デバイスのミックスを取得
  - per-app(複数): `Windows 10 2004+` のプロセスループバック(`ActivateAudioInterfaceAsync` + `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK` + `IAudioCaptureClient`)で対象PIDの音声のみを取得。複数選択時はPID数分のキャプチャクライアントを並列でポーリング
  - マイク: `eCapture` デバイスから通常キャプチャ。`enabled` フラグでON/OFF (`F-AU-03`)
- **フォーマット:** 全て `48kHz Float32 Stereo` にリサンプル( `rubato` or `samplerate` crate) してからミキシング。Rust側で `f32` でミックス

#### 3.2.2 Linux (PipeWire)

- **列挙:** `pw_registry` で `PW_TYPE_INTERFACE_Node` をフィルタ。`media.class == Stream/Output/Audio` かつ `application.process.binary` が存在するものが per-app。`node.description` を表示名に
- **キャプチャ:**
  - システム全体: `default` sink の `monitor` から取得
  - per-app(複数): 選択されたnodeの `target.object` を指定して `pw_stream_connect`。複数時は複数 `Stream` を生成
  - マイク: `media.class == Stream/Input/Audio` の source
- **マイクON/OFF:** `F-AU-03` の `enabled/muted` はMixerのゲートで処理し、`Stream` の接続断は行わない(再接続はレイテリとnode再探索のコストが不要なため)

#### 3.2.3 音声ミキシング (Rust側で完結)

> v0.3の教訓: FFmpegの `amix` に頼らず、Rustで1本のPCMに合成してからFFmpegへ渡す。FFmpegへの音声入力は常に1本(`f32le 48kHz stereo` named pipe) に統一。WASAPI/PipeWireのネイティブ形式が f32 なため、**f32のままキャプチャ→加算→パイプ**し、s16⇄f32の変換往復をなくす

```
[Chrome PCM 48k f32] ─┐
[Spotify PCM 48k f32] ─┼─> Rust Mixer (f32加算 + clamp + ゲイン) ─> f32le pipe ─> FFmpeg -f f32le -ar 48000 -ac 2 -i ...
[Mic PCM 48k f32] ─────┘        ↕ VU計算(peak/rms) はここで算出し event でReactへ
```

- **ゲイン/ミュート:** 各ソースに `gain: f32 (0.0-2.0)` と `muted: bool`。`VU` は `rms` と `peak` を 50ms ごとに計算し `audio://vu` event で送信
- **複数アプリの加算:** `f32` で加算後 `tanh` ソフトクリップ or 単純 `clamp(-1.0,1.0)`。MVPは `clamp` で十分
- **リサンプル:** `rubato` で全ソースを `48kHz` に統一

---

## 4. FFmpeg連携設計 (修正版)

### 4.1 方針

- **FFmpegの責務はエンコード + RTMP送信のみ。** キャプチャはRustが担う
- **映像・音声ともにRust→FFmpegへは named pipe (OSパイプ) 2本で渡す**(方式はこれ1つ):
  - 映像: `rawvideo` (BGRA, プロファイル w×h 正規化済み, §3.1.3)
  - 音声: `f32le` 48kHz 2ch (Rust Mixer合成済み, §3.2.3)

> Tauri sidecarの `stdin` は1本しか使えないため、映像+音声の2本は named pipe で渡す。Windows `\\.\pipe\ezTopaz_video` / `\\.\pipe\ezTopaz_audio`, Linux `/tmp/ezTopaz_video.pipe` / `/tmp/ezTopaz_audio.pipe`。Rustがパイプサーバを先に生成し、FFmpegをクライアントとして接続させる。**再接続(リトライ)時はFFmpegを再spawnするため、Rust側もパイプインスタンスを再生成する**(§9)

### 4.2 パイプ方式

```
Rust CaptureManager
   ├─ video_tx ─> NamedPipe "ezTopaz_video" ─> FFmpeg -f rawvideo -pix_fmt bgra -s {profile.w}x{profile.h} -r {profile.fps} -i \\.\pipe\ezTopaz_video
   └─ audio_tx ─> NamedPipe "ezTopaz_audio" ─> -f f32le -ar 48000 -ac 2 -i \\.\pipe\ezTopaz_audio
                                                         ─> -c:v ... -c:a aac -f flv rtmp://...
```

- **映像フォーマット:** `BGRA` (WGC/Portalのネイティブ)。`rawvideo` でFFmpegへ。サイズはプロファイル正規化済みのため起動中不変(§3.1.3)
- **音声フォーマット:** `f32le` 48kHz 2ch (Mixer合成済み)
- **エンコード:** パイプから受けたフレームをFFmpegが `libx264` / `h264_nvenc` 等でエンコード。BGRA→YUV420PのswscaleはCPU仕事なので、コストはスパイクで実測し超過ならGPU側NV12変換へ(§15)
- **ステータス:** `-progress pipe:1` をSupervisorがパースし `bitrate_kbps` / `dropped_frames` を算出 → `stream://status` event

### 4.3 FFmpegコマンド生成

```rust
fn build_ffmpeg_args(cfg: &StreamConfig, video_pipe: &str, audio_pipe: &str) -> Vec<String> {
  let (w,h,fps,v_kbps,a_kbps) = cfg.profile.to_params();
  let gop = fps * 2; // 2秒
  let vcodec = match cfg.encoder_override {
    Auto => probe_best_encoder(), // nvenc > qsv > amf(Win) > vaapi(Linux) > libx264 (vulkanは手動のみ, §8.1)
    Manual(e) => e,
  };
  // presetとレートコントロールはエンコーダ毎に異なる (p1はNVENC系の名前でqsv/amf/vaapiには渡せない)
  let (preset, rc): (Option<&str>, &[&str]) = match vcodec.as_str() {
    "h264_nvenc" => (Some("p1"), &["-rc","cbr"]),
    "libx264"    => (Some("veryfast"), &["-rc-lookahead","0"]),
    "h264_qsv"   => (Some("veryfast"), &[]),
    "h264_amf"   => (Some("speed"), &[]),
    _            => (None, &[]), // vaapi/vulkan: preset指定なし
  };
  let mut args = vec![
    "-f","rawvideo","-pix_fmt","bgra","-s", &format!("{w}x{h}"),"-r",&fps.to_string(),"-i", video_pipe,
    "-f","f32le","-ar","48000","-ac","2","-i", audio_pipe,
    "-c:v", &vcodec, "-pix_fmt","yuv420p","-profile:v","high","-bf","0","-g",&gop.to_string(),
    "-b:v",&format!("{v_kbps}k"),"-maxrate",&format!("{v_kbps}k"),"-bufsize",&format!("{}k",v_kbps*2),
  ];
  if let Some(p) = preset { args.extend(["-preset", p]); }
  args.extend_from_slice(rc); // AC-05 (2000k±10%) をffprobe計測で満たすためレートコントロールを明示
  args.extend([
    "-c:a","aac","-b:a",&format!("{a_kbps}k"),"-ac","2","-ar","48000",
    "-f","flv", &format!("{}/{}", cfg.ingest_url.trim_end_matches('/'), cfg.stream_key)
  ]);
  args
}
```

- **GOP:** `fps*2` で2秒固定 (`F-EN-05`)
- **上限ガード:** `F-EN-04` で `v_kbps>2000` or `a_kbps>320` はRust側で `Err(OverBitrate)` を返し起動前にUIで赤表示
- **probe:** `ffmpeg -encoders` のパースに加え、候補ごとの1フレーム書き出しテストを実施(§8.1)。初回起動時にキャッシュ

### 4.4 同梱FFmpegビルド要件

- **取得:** **キャプチャはRust側に全て寄せたためFFmpegにキャプチャデバイスは不要**。公式GPL事前ビルド(BtbN releases等の**pinned安定版**。`master` は使わない)を `resources/ffmpeg/` に同梱し、`ffmpeg-sidecar` は同梱パスを参照。CIでのセルフビルドは行わない(要件§4.4のデバイス有効化要件は本設計により不要化 → requirements v0.3.2で注記)。必要機能: `rawvideo/f32le` 入力, `libx264`, `aac`, `h264_nvenc/qsv/amf/vaapi`, `flv`, `rtmp`
- **GPL:** `libx264` を含むGPLビルド。`LICENSES/ffmpeg-GPL.txt` とソースURLを `README` に明記 (`requirements.md:371`)
- **サイズ:** `Win <100MB`, `Linux <100MB` はインストーラ圧縮後。`strip` で25MB程度を目標(`upx` はAV誤検知リスクがあるため任意扱い)

---

## 5. Tauri IPC設計

### 5.1 Commands (invoke)

```rust
#[tauri::command] fn get_displays() -> Result<Vec<Display>, String>
#[tauri::command] fn get_windows() -> Result<Vec<WindowInfo>, String> // Portal環境では空 (ピッカーは start_portal_picker)
#[tauri::command] fn start_portal_picker() -> Result<ScreenTarget, String> // Portalのピッカーを開き、選択結果(display/window id)を返す
#[tauri::command] fn get_audio_devices() -> Result<AudioDevices, String> // { inputs, outputs, apps }
#[tauri::command] fn get_profiles() -> Result<ProfilesConfig, String>
#[tauri::command] fn save_profiles(cfg: ProfilesConfig) -> Result<(), String>
#[tauri::command] fn probe_encoders() -> Result<Vec<EncoderInfo>, String>
#[tauri::command] fn start_stream(cfg: StreamConfig) -> Result<(), String>
#[tauri::command] fn stop_stream() -> Result<(), String>
#[tauri::command] fn update_audio_mix(mix: AudioMixUpdate) -> Result<(), String> // 配信中のゲイン/ミュート/マイクON-OFFをMixerへ即時反映 (F-AU-03/04)
#[tauri::command] fn get_status() -> Result<StreamStatus, String>
#[tauri::command] fn copy_to_clipboard(text: String) -> Result<(), String>
#[tauri::command] fn open_logs_dir() -> Result<(), String> // F-CF-04 「ログを開く」
```

### 5.2 Events (listen)

```rust
emit("stream://status", StreamStatus { is_live, duration_sec, bitrate_kbps, dropped_frames })
emit("stream://vu", VuMeter { app_levels: HashMap<String,f32>, mic_level: f32, master_level: f32 })
emit("stream://preview", PreviewFrame { base64_png, w, h })
emit("stream://log", LogLine { level, msg })
emit("stream://error", StreamError { code, msg })
```

### 5.3 型定義

```ts
// shared via tauri-specta or manual
type StreamConfig = {
  ingestUrl: string // default rtmp://topaz.chat/live
  streamKey: string
  screen: { type: "display"|"window", id: string }
  audio: { mode: "system"|"apps", apps: string[], mic: { device: string, enabled: boolean, muted: boolean, gain: number } }
  profile: ProfileId // low | mid | high | 1080p | custom
  encoderOverride: "auto" | "libx264" | "h264_nvenc" | "h264_qsv" | "h264_amf" | "h264_vaapi" | "h264_vulkan"
}
type Profile = { name: string, w:number, h:number, fps:number, v_kbps:number, a_kbps:number, encoder:"auto" }
type AudioMixUpdate = {
  apps: Record<string, { gain: number, muted: boolean }> // アプリ別 (F-AU-04)
  mic: { enabled: boolean, muted: boolean, gain: number } // F-AU-03
}
```

---

## 6. React UI設計

### 6.1 画面構成

```
App
 ├─ Header (ezTopaz | ●00:12 | ja/en | ⚙)
 ├─ Main (tabs: Screen | Audio | Output)
 │   ├─ ScreenSelector: Radio[Display/Window] + DisplayGrid + WindowList + PreviewCanvas(16:9)
 │   ├─ AudioSelector: Radio[System/Apps] + AppMultiSelect(checkbox) + MicSelect + VuMeter * N
 │   ├─ ProfileSelector: [Low][Mid●][High][1080p⚠] + EncoderSelect(auto/...)
 │   └─ StreamControl: IngestInput(editable) + KeyInput + UrlCopy(PC/Quest) + BigToggle
 └─ SettingsModal (Profiles CRUD + EncoderDetails + Logs + Licenses)
```

### 6.2 状態管理 (zustand)

```ts
store = {
  displays, windows, audioDevices,
  selectedScreen, selectedAudio, profiles, activeProfile,
  encoderOverride, ingestUrl, streamKey,
  isLive, status, vu,
  locale: "ja"|"en"
}
```

### 6.3 コンポーネント詳細

- **ScreenSelector:** `get_displays()` で取得、`DisplayGrid` でサムネ+解像度表示。Windowは `get_windows()` で取得、Portal環境では空なので「Portalピッカーで選択」ボタン → `start_portal_picker()`。選択結果は配信開始まで保持
- **AudioSelector:** `mode` が `system` なら `AppMultiSelect` は disabled。`apps` は `get_audio_devices().apps` から複数選択。`VuMeter` は `stream://vu` で 50ms 更新。ゲイン/ミュート/マイクON-OFFの変更は配信中 `update_audio_mix()` で即時反映(配信前はローカルstate、開始時に `start_stream` へ)
- **ProfileSelector:** 4ボタンは `activeProfile` でハイライト。`1080p` は `warn` ツールチップ。`EncoderSelect` は `probe_encoders()` で有効なもののみ表示、無効は disabled + 理由
- **StreamControl:** `streamKey` 入力と同時に `rtspt://topaz.chat/live/{key}` と `rtsp://...` を生成。`copy_to_clipboard` でコピー、トースト表示。`IngestInput` はデフォ `rtmp://topaz.chat/live`、編集可
- **BigToggle:** `isLive` で `灰:配信開始` / `赤:配信停止`。`start_stream` 失敗時は `stream://error` でモーダル

### 6.4 プレビュー

- Rust側で `BGRA` フレームを 1fps に間引き、`png` エンコードして `base64` で `preview` event。React側は `<img src={preview.base64}>` で表示。配信前のみ動作、配信中は `StreamStatus` のサムネで代用

---

## 7. 設定・永続化

### 7.1 ファイル

- `Win: %APPDATA%/ezTopaz/profiles.json`
- `Linux: ~/.config/ezTopaz/profiles.json` (XDG)
- `logs/: ~/.config/ezTopaz/logs/ezTopaz-YYYY-MM-DD.log` (FFmpeg stderr 垂れ流し)

### 7.2 スキーマ

```json
{
  "version": 2,
  "locale": "ja",
  "ingestUrl": "rtmp://topaz.chat/live",
  "activeProfile": "mid",
  "profiles": {
    "low":  { "name":"低画質", "w":854, "h":480, "fps":30, "v_kbps":800, "a_kbps":128, "encoder":"auto" },
    "mid":  { "name":"中画質", "w":1280,"h":720, "fps":30, "v_kbps":1500, "a_kbps":192, "encoder":"auto" },
    "high": { "name":"高画質", "w":1280,"h":720, "fps":60, "v_kbps":2000, "a_kbps":320, "encoder":"auto" },
    "1080p":{ "name":"1080p", "w":1920,"h":1080,"fps":30, "v_kbps":2000, "a_kbps":320, "encoder":"auto", "warn":"Topaz上限2000kのため720p推奨" }
  },
  "lastStreamKey": "my-key",
  "lastSources": { "screen": {"type":"display","id":"0"}, "includeApps":["Chrome","Spotify"], "mic":{"device":"default","enabled":true,"gain":1.0} },
  "encoderOverride": "auto"
}
```

- 起動時に `ConfigManager::load()` で読み込み、存在しなければデフォ生成
- 保存は `save_profiles` で `atomic write` (tmp→rename。Windowsは宛先既存でrename失敗するため `fs::replace` を使用)

---

## 8. エンコーダ設計

### 8.1 判定ロジック

```rust
fn probe_usable() -> Vec<String> {
  // -encoders は「コンパイル済み」しか返さずドライバ有無を反映しないため、
  // 候補ごとに1フレームの書き出しテストを行い、実際に動くものだけを返す
  // test_encode_1frame: ffmpeg -f lavfi -i testsrc=duration=0.1:size=320x240:rate=30 -c:v {e} -f null - が exit 0 か
  ["h264_nvenc","h264_qsv","h264_amf","h264_vaapi"].iter()
    .filter(|e| test_encode_1frame(e)).cloned().collect()
}

fn probe_best() -> String {
  let usable = probe_usable(); // 初回起動時にキャッシュ
  if usable.contains(&"h264_nvenc") { return "h264_nvenc" } // nvencに -hwaccel cuda は不要(エンコーダはnvenc APIを直接叩く)
  if usable.contains(&"h264_qsv") { return "h264_qsv" }
  if target_os=="windows" && usable.contains(&"h264_amf") { return "h264_amf" }
  if target_os=="linux" && usable.contains(&"h264_vaapi") { return "h264_vaapi" }
  "libx264" // vulkanは自動では選ばない(手動のみ。要件F-EN-03「手動選択でvulkan可」)。昇格可否は§15
}
```

- **自動:** 上記順。`vulkan` は自動では `vaapi` があれば選ばない。`vaapi` が無く `vulkan` があれば選ぶ
- **手動:** UIで `auto` 以外を選んだ場合はそのまま使う。無効なエンコーダを選んだら `Err(EncoderNotAvailable)` を返しUIで赤表示

### 8.2 プロファイル適用

- `w,h,fps` は映像パイプのサイズ/レートに、`v_kbps/a_kbps` はFFmpegの `-b:v/-b:a` に
- `1080p` は `warn` をUIで表示、エンコード自体は `2000k` で実行。視聴側でブロックノイズが出る旨をツールチップで明記

---

## 9. エラーハンドリング

| エラー | 検出 | UI表示 |
|---|---|---|
| X11検出 | 起動時 `XDG_SESSION_TYPE==x11` | モーダル「Waylandで起動してください」+ 配信開始無効 |
| PipeWire未導入 | `pipewire` crate connect失敗 | トースト「PipeWireが見つかりません。システム音声+マイクのみで動作します」+ `AppMultiSelect` disabled |
| デバイス未接続 | `start_stream` 前に `selected` が `get_devices` に無い | インライン赤「デバイスが見つかりません」 |
| ビットレート超過 | `cfg.v_kbps>2000` or `a_kbps>320` | インライン赤「Topaz上限を超えています」+ 開始無効 |
| FFmpeg起動失敗 | `supervisor.spawn` 失敗 | `stream://error` でモーダル + logsリンク |
| 配信切断 | FFmpeg stderr `Connection reset` or `Error number -32` | `F-ST-04` で3回リトライ(指数バックオフ)、UIで「再接続中 1/3」表示。3回失敗で停止。**再spawn時にRust側named pipeサーバも再生成**する(§4.1) |
| StreamKey空/不正 | `key.len()<3` or 正規表現外 | インライン赤 |

- **ログ:** FFmpegの `stderr` を `logs/` に追記、UIの `LogView` で `tail -n 100` 表示。`stream://log` でリアルタイム流し
- **プロセス後始末:** SupervisorはDrop時およびアプリ終了時にFFmpegを確実にkill(要件§6「クラッシュ時FFmpeg確実kill」)。WinはJob Objectで子プロセスを紐付け、Linuxはプロセスグループkill

---

## 10. パフォーマンス・リソース

| 項目 | 目標 | 対策 |
|---|---|---|
| CPU (720p30 x264) | <15% | Rustで `BGRA→YUV` 変換は `ffmpeg` に任せる。`rubato` リサンプルは `f32` で軽量 |
| CPU (NVENC) | <5% | WGC+GPUエンコードでCPU解放。ただし BGRA→YUV420P のswscaleはCPU仕事(1080p30で片コア級)。スパイクで実測し超過ならGPU側NV12変換へ(§15) |
| メモリ | <300MB | フレームは `tokio::mpsc` で1フレームバッファ。ログはローテート(10MB) |
| 起動時間 | <2秒 | `probe` は初回のみ、以降キャッシュ。Reactは `vite` で `code split` なし |
| バンドル | <100MB (圧縮前) → 実測 <50MB 目標 | FFmpeg `strip` + `upx` (Win), Linuxは `deb` で `ffmpeg` を外部依存にすることも検討(将来) |

---

## 11. セキュリティ・ライセンス

- **画面共有権限:** Portal/WGCのOS標準ダイアログ経由のみ。権限はアプリが要求しない
- **StreamKey:** 平文保存だが公開情報(視聴URLの一部)なので暗号化不要。`README` に明記
- **FFmpegライセンス:** `libx264` 有効時はGPL。`LICENSES/ffmpeg-GPL.txt` + `https://ffmpeg.org/download.html` のソース取得方法を `Settings > Licenses` と `README` に記載。MIT本体とは分離
- **Tauri:** `tauri.conf.json` で `csp: default-src 'self'`、外部URLは `rtspt://` のコピーのみで通信はRTMPのみ

---

## 12. テスト設計

### 12.1 単体

- `config::tests` — JSON roundtrip, 上限ガード, マイグレーション
- `ffmpeg::probe::tests` — `encoders` パース, 判定順, 1フレーム書き出しテストのモック
- `audio::mixer::tests` — f32加算, `clamp`, `gain`, VU計算
- React: `vitest` で `ProfileSelector` の `1080p` 警告表示, `copy` 機能

### 12.2 結合

- `capture::screen` — WGC/Portalで1フレーム取得できること (手動, Wayland実機)
- `capture::audio` — `Chrome` の音だけが取得できること (手動, VUで確認)
- `supervisor` — `ffmpeg` 起動→2秒で `isLive` が `true`、 `ffprobe` で `2000k±10%` になること

### 12.3 受入 (requirements.md:378 `AC-01`〜`10`)

| AC | 自動/手動 |
|---|---|
| `AC-01` 未設定で開始不可 | 自動 (vitest) |
| `AC-04` Spotifyのみ | 手動 (実機 + ヘッドホン) |
| `AC-04c` 複数アプリ | 手動 |
| `AC-05` 高画質 2000k | 自動 (CIでffprobe) |
| `AC-08` Win2004+/Ubuntu Wayland/Arch Wayland | 手動 (実機 Topaz Playerで視聴) |

---

## 13. ビルド・配布

### 13.1 ローカル

```bash
pnpm i && pnpm tauri dev          # 開発
pnpm tauri build                  # リリース (resources/ffmpeg を同梱)
```

### 13.2 CI (GitHub Actions)

```yaml
jobs:
  build:
    strategy: { matrix: { os: [windows-latest, ubuntu-22.04] } }
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: pnpm i && pnpm tauri build   # FFmpegはpinned公式ビルドをresources/へ配置(リポジトリ同梱 or CIキャッシュ)
      - uses: actions/upload-artifact@v4
        with: { path: "src-tauri/target/release/bundle/*" }
```

- **Win:** `NSIS` (`src-tauri/bundle/nsis`), `msi` (WiX)
- **Linux:** `AppImage` + `deb` (+ `AUR` は手動)
- **FFmpeg:** セルフビルド不要(§4.4)。pinned公式GPLビルドをリポジトリにコミット or CIキャッシュから `resources/ffmpeg/` へ配置

### 13.3 配布

- GitHub Releases + BOOTH (手動DL)。自動更新なし (`requirements.md:285`)
- `README` に `Topaz FANBOX` 支援リンクを明記

---

## 14. 実装順序 (WBS 骨子)

1. **スパイク (1週):** WGC 1フレーム取得 / Portal+PipeWire 1フレーム / WASAPI per-app 1秒取得 / PipeWire node列挙 / named pipeでFFmpegへrawvideo+f32leを流すPoC / FramePacer(最終フレーム複製+スケール) / swscale(BGRA→YUV420P)コスト計測 / エンコーダ1フレーム書き出しテスト
2. **基盤 (1週):** Tauri+React雛形, ConfigManager, probe, IPC骨組み
3. **画面 (1週):** ScreenSelector + Preview + 全画面/ウィンドウ切替
4. **音声 (1.5週):** AudioSelector(複数) + Mixer + VU
5. **配信 (1.5週):** FFmpegSupervisor + Profile + Ingest可変 + URLコピー + エラーハンドリング
6. **仕上げ (1週):** i18n, ログ, 設定モーダル, AC手動テスト, ドキュメント

---

## 15. 未決事項

- `vulkan` を自動判定に昇格するかはスパイクの `vaapi vs vulkan` 計測後(現行は手動のみ)
- BGRA→YUV420P のswscaleコストが目標(CPU<15%/NVENC時<5%)を超える場合、GPU側NV12変換(D3D11 VideoProcessor等)へ切替、全画面のみ `ddagrab` へ後退するかもスパイク実測後
- Linux `deb` で `ffmpeg` を外部依存にするか同梱するかはサイズ実測後

---

## 16. 変更履歴

| 版 | 日付 | 変更 |
|---|---|---|
| 0.1 | 2026-09-02 | 初版作成 |
| 0.2 | 2026-09-02 | レビュー修正: FramePacer+解像度正規化(§3.1.3)、エンコーダ別preset/レートコントロール(§4.3)、probeに1フレーム実機テスト(§8.1)、FFmpegはpinned公式GPLビルドでCIセルフビルド廃止(§4.4/§13.2)、パイプ方式1本化(§4.1)、配信中ミックス更新等のIPC追加(§5.1)、f32le統一、再接続時パイプ再生成(§9)、API名修正、その他軽微修正 |

