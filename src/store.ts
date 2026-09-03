import { create } from "zustand";
import { api } from "./lib/api";
import type {
  AudioDevices,
  Display,
  EncoderInfo,
  MicSource,
  ProfilesConfig,
  ScreenTarget,
  StreamStatus,
  VuMeter,
  WindowInfo,
} from "./lib/types";

export type Tab = "screen" | "audio" | "profile";

interface AppState {
  tab: Tab;
  setTab: (t: Tab) => void;

  // data from backend
  displays: Display[];
  windows: WindowInfo[];
  audioDevices: AudioDevices | null;
  encoders: EncoderInfo[];
  profiles: ProfilesConfig | null;

  // selections (persisted via save_profiles)
  screen: ScreenTarget;
  audioMode: "system" | "apps";
  selectedApps: string[];
  mic: MicSource;
  profileId: string;
  encoderOverride: string;
  ingestUrl: string;
  streamKey: string;

  // runtime
  status: StreamStatus;
  isLive: boolean;
  previewing: boolean;
  toast: string | null;
  backendError: string | null;
  preview: string | null;
  vu: VuMeter;
  setPreview: (url: string | null) => void;
  refreshVu: () => Promise<void>;

  loadAll: () => Promise<void>;
  setScreen: (s: ScreenTarget) => void;
  setAudioMode: (m: "system" | "apps") => void;
  toggleApp: (id: string) => void;
  setMic: (m: Partial<MicSource>) => void;
  setProfileId: (id: string) => void;
  setEncoderOverride: (e: string) => void;
  setIngestUrl: (u: string) => void;
  setStreamKey: (k: string) => void;
  setProfiles: (p: ProfilesConfig) => void;
  showToast: (msg: string) => void;
  refreshStatus: () => Promise<void>;
  startStream: () => Promise<void>;
  stopStream: () => Promise<void>;
  startPreview: () => Promise<void>;
  stopPreview: () => Promise<void>;
}

const emptyStatus: StreamStatus = {
  isLive: false,
  durationSec: 0,
  bitrateKbps: 0,
  droppedFrames: 0,
  retrying: null,
};

// Preview screen-switch restart (fixes "stuck on previous screen").
// setScreen() clears the stale frame immediately and re-issues start_preview
// with the new target. Rapid switches are debounced so only the last
// selection hits the backend; the generation counter drops stale callbacks
// (e.g. user pressed stop while a restart was in flight).
let previewRestartTimer: ReturnType<typeof setTimeout> | null = null;
let previewRestartGen = 0;

function cancelPendingPreviewRestart() {
  previewRestartGen += 1;
  if (previewRestartTimer !== null) {
    clearTimeout(previewRestartTimer);
    previewRestartTimer = null;
  }
}

export const useStore = create<AppState>((set, get) => ({
  tab: "screen",
  setTab: (t) => set({ tab: t }),

  displays: [],
  windows: [],
  audioDevices: null,
  encoders: [],
  profiles: null,

  screen: { type: "display", id: "" },
  audioMode: "system",
  selectedApps: [],
  mic: { device: "default", enabled: true, muted: false, gain: 1.0 },
  profileId: "mid",
  encoderOverride: "auto",
  ingestUrl: "rtmp://topaz.chat/live",
  streamKey: "",

  status: emptyStatus,
  isLive: false,
  previewing: false,
  toast: null,
  backendError: null,
  preview: null,
  vu: { apps: {}, mic: null, master: { peak: 0, rms: 0 } },

  async loadAll() {
    const [displays, windows, audioDevices, encoders, profiles] = await Promise.allSettled([
      api.getDisplays(),
      api.getWindows(),
      api.getAudioDevices(),
      api.probeEncoders(),
      api.getProfiles(),
    ]);
    const backendError =
      displays.status === "rejected" ? String(displays.reason) : null;
    const cfg = profiles.status === "fulfilled" ? profiles.value : null;
    set({
      displays: displays.status === "fulfilled" ? displays.value : [],
      windows: windows.status === "fulfilled" ? windows.value : [],
      audioDevices: audioDevices.status === "fulfilled" ? audioDevices.value : null,
      encoders: encoders.status === "fulfilled" ? encoders.value : [],
      profiles: cfg,
      backendError,
      ...(cfg
        ? {
            ingestUrl: cfg.ingestUrl,
            streamKey: cfg.lastStreamKey,
            profileId: cfg.activeProfile,
            encoderOverride: cfg.encoderOverride,
            screen: cfg.lastSources.screen,
            selectedApps: cfg.lastSources.includeApps,
            mic: cfg.lastSources.mic,
            audioMode: cfg.lastSources.includeApps.length > 0 ? "apps" : "system",
          }
        : {}),
    });
  },

  setScreen: (screen) => {
    const prev = get().screen;
    if (prev.type === screen.type && prev.id === screen.id) return;
    const wasPreviewing = get().previewing && !get().isLive;
    // Drop the stale frame at once; otherwise the UI keeps showing the
    // previous screen until (or unless) a new frame arrives.
    set(wasPreviewing ? { screen, preview: null } : { screen });
    if (!wasPreviewing) return;
    const gen = ++previewRestartGen;
    if (previewRestartTimer !== null) {
      clearTimeout(previewRestartTimer);
    }
    previewRestartTimer = setTimeout(() => {
      previewRestartTimer = null;
      // A newer switch / stop supersedes this one.
      if (gen !== previewRestartGen) return;
      if (!get().previewing || get().isLive) return;
      const s = get();
      void api
        .startPreview({
          ingestUrl: s.ingestUrl,
          streamKey: s.streamKey,
          screen: s.screen,
          audio: { mode: s.audioMode, apps: s.selectedApps, mic: s.mic },
          profileId: s.profileId,
          encoderOverride: s.encoderOverride,
        })
        .then(() => {
          if (gen === previewRestartGen) {
            set({ previewing: true });
          } else if (!get().previewing) {
            // Stopped while the restart was in flight: don't leave it running.
            api.stopPreview().catch(() => undefined);
          }
        })
        .catch((e) => {
          if (gen !== previewRestartGen) return;
          set({ previewing: false, backendError: String(e) });
          get().showToast(String(e));
        });
    }, 250);
  },
  setAudioMode: (audioMode) => set({ audioMode }),
  toggleApp: (id) =>
    set((s) => ({
      selectedApps: s.selectedApps.includes(id)
        ? s.selectedApps.filter((a) => a !== id)
        : [...s.selectedApps, id],
    })),
  setMic: (m) => set((s) => ({ mic: { ...s.mic, ...m } })),
  setProfileId: (profileId) => set({ profileId }),
  setEncoderOverride: (encoderOverride) => set({ encoderOverride }),
  setIngestUrl: (ingestUrl) => set({ ingestUrl }),
  setStreamKey: (streamKey) => set({ streamKey }),

  setProfiles: (profiles) => {
    set({ profiles });
    void api.saveProfiles(profiles).catch(() => undefined);
  },

  setPreview: (preview) => set({ preview }),

  async refreshVu() {
    try {
      const vu = await api.getVu();
      set({ vu });
    } catch {
      /* backend absent */
    }
  },

  showToast: (msg) => {
    set({ toast: msg });
    setTimeout(() => set((s) => (s.toast === msg ? { toast: null } : {})), 2000);
  },

  async refreshStatus() {
    try {
      const status = await api.getStatus();
      set({ status, isLive: status.isLive });
    } catch {
      /* backend absent in browser dev */
    }
  },

  async startStream() {
    cancelPendingPreviewRestart();
    const s = get();
    try {
      const status = await api.startStream({
        ingestUrl: s.ingestUrl,
        streamKey: s.streamKey,
        screen: s.screen,
        audio: { mode: s.audioMode, apps: s.selectedApps, mic: s.mic },
        profileId: s.profileId,
        encoderOverride: s.encoderOverride,
      });
      set({ status, isLive: status.isLive, previewing: false });
    } catch (e) {
      set({ backendError: String(e) });
    }
  },

  async stopStream() {
    cancelPendingPreviewRestart();
    try {
      await api.stopStream();
    } finally {
      set({ isLive: false, status: emptyStatus, previewing: false });
    }
  },

  async startPreview() {
    const s = get();
    try {
      await api.startPreview({
        ingestUrl: s.ingestUrl,
        streamKey: s.streamKey,
        screen: s.screen,
        audio: { mode: s.audioMode, apps: s.selectedApps, mic: s.mic },
        profileId: s.profileId,
        encoderOverride: s.encoderOverride,
      });
      set({ previewing: true });
    } catch (e) {
      set({ backendError: String(e) });
    }
  },

  async stopPreview() {
    cancelPendingPreviewRestart();
    try {
      await api.stopPreview();
    } finally {
      set({ previewing: false, preview: null });
    }
  },
}));
