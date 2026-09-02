// Mirror of eztopaz-core/src/ipc_types.rs (camelCase over Tauri IPC)

export type ScreenTarget = { type: "display" | "window"; id: string };

export interface Profile {
  name: string;
  w: number;
  h: number;
  fps: number;
  v_kbps: number;
  a_kbps: number;
  encoder: string;
  warn?: string;
}

export interface MicSource {
  device: string;
  enabled: boolean;
  muted: boolean;
  gain: number;
}

export interface ProfilesConfig {
  version: number;
  locale: string;
  ingestUrl: string;
  activeProfile: string;
  profiles: Record<string, Profile>;
  lastStreamKey: string;
  lastSources: { screen: ScreenTarget; includeApps: string[]; mic: MicSource };
  encoderOverride: string;
}

export interface StreamConfig {
  ingestUrl: string;
  streamKey: string;
  screen: ScreenTarget;
  audio: { mode: string; apps: string[]; mic: MicSource };
  profileId: string;
  encoderOverride: string;
}

export interface StreamStatus {
  isLive: boolean;
  durationSec: number;
  bitrateKbps: number;
  droppedFrames: number;
  retrying: number | null;
}

export interface Display {
  id: string;
  label: string;
  w: number;
  h: number;
}

export interface WindowInfo {
  id: string;
  title: string;
  app: string;
}

export interface DeviceInfo {
  id: string;
  label: string;
  isDefault: boolean;
}

export interface AppAudio {
  id: string;
  label: string;
}

export interface AudioDevices {
  inputs: DeviceInfo[];
  outputs: DeviceInfo[];
  apps: AppAudio[];
}

export interface EncoderInfo {
  name: string;
  usable: boolean;
  reason: string | null;
}

export interface VuLevel {
  peak: number;
  rms: number;
}

export interface VuMeter {
  apps: Record<string, VuLevel>;
  mic: VuLevel | null;
  master: VuLevel;
}

export interface PreviewFrame {
  dataUrl: string;
  w: number;
  h: number;
}

export const MAX_VIDEO_KBPS = 2000;
export const MAX_AUDIO_KBPS = 320;
