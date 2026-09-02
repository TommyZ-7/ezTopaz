// Thin Tauri invoke wrapper. In a plain browser (vite dev without tauri),
// commands reject with a clear error instead of crashing.

import { invoke } from "@tauri-apps/api/core";
import type {
  AudioDevices,
  Display,
  EncoderInfo,
  ProfilesConfig,
  ScreenTarget,
  StreamConfig,
  StreamStatus,
  VuMeter,
  WindowInfo,
} from "./types";

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function cmd<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  if (!inTauri) {
    return Promise.reject(new Error(`"${name}" requires the desktop app (Tauri)`));
  }
  return invoke<T>(name, args);
}

export const api = {
  ping: () => cmd<string>("ping"),
  getDisplays: () => cmd<Display[]>("get_displays"),
  getWindows: () => cmd<WindowInfo[]>("get_windows"),
  startPortalPicker: () => cmd<ScreenTarget>("start_portal_picker"),
  getAudioDevices: () => cmd<AudioDevices>("get_audio_devices"),
  getProfiles: () => cmd<ProfilesConfig>("get_profiles"),
  saveProfiles: (cfg: ProfilesConfig) => cmd<void>("save_profiles", { cfg }),
  probeEncoders: () => cmd<EncoderInfo[]>("probe_encoders"),
  startStream: (cfg: StreamConfig) => cmd<StreamStatus>("start_stream", { cfg }),
  stopStream: () => cmd<void>("stop_stream"),
  getStatus: () => cmd<StreamStatus>("get_status"),
  getVu: () => cmd<VuMeter>("get_vu"),
  updateAudioMix: (mix: object) => cmd<void>("update_audio_mix", { mix }),
  copyToClipboard: (text: string) => cmd<void>("copy_to_clipboard", { text }),
  openLogsDir: () => cmd<void>("open_logs_dir"),
};
