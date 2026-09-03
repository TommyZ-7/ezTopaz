import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Display, EncoderInfo } from "./lib/types";

const startPreviewMock = vi.fn(async (_cfg: unknown) => undefined);
const stopPreviewMock = vi.fn(async () => undefined);
const getDisplaysMock = vi.fn<() => Promise<Display[]>>(async () => []);
const getWindowsMock = vi.fn(async () => []);
const getAudioDevicesMock = vi.fn(async () => null);
const getProfilesMock = vi.fn(async () => null);
const probeEncodersMock = vi.fn<() => Promise<EncoderInfo[]>>(async () => []);

vi.mock("./lib/api", () => ({
  api: {
    startPreview: (cfg: unknown) => startPreviewMock(cfg),
    stopPreview: () => stopPreviewMock(),
    getDisplays: () => getDisplaysMock(),
    getWindows: () => getWindowsMock(),
    getAudioDevices: () => getAudioDevicesMock(),
    getProfiles: () => getProfilesMock(),
    probeEncoders: () => probeEncodersMock(),
  },
}));

import { useStore } from "./store";

beforeEach(() => {
  vi.clearAllMocks();
  vi.clearAllTimers();
  vi.useRealTimers();
  useStore.setState({
    screen: { type: "display", id: "display:0" },
    previewing: false,
    isLive: false,
    preview: null,
    ingestUrl: "rtmp://topaz.chat/live",
    streamKey: "k",
    audioMode: "system",
    selectedApps: [],
    mic: { device: "default", enabled: true, muted: false, gain: 1.0 },
    profileId: "mid",
    encoderOverride: "auto",
    booted: false,
    encodersLoading: true,
    encoders: [],
    displays: [],
  });
});

describe("setScreen during preview", () => {
  it("clears stale frame and restarts preview with the new target", async () => {
    vi.useFakeTimers();
    useStore.setState({
      previewing: true,
      preview: "data:old",
      screen: { type: "display", id: "display:0" },
    });

    useStore.getState().setScreen({ type: "display", id: "display:1" });

    // stale frame dropped immediately so the old screen doesn't linger
    expect(useStore.getState().screen.id).toBe("display:1");
    expect(useStore.getState().preview).toBeNull();
    expect(startPreviewMock).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(250);
    expect(startPreviewMock).toHaveBeenCalledTimes(1);
    const cfg = startPreviewMock.mock.calls[0][0] as unknown as { screen: { id: string } };
    expect(cfg.screen.id).toBe("display:1");
    expect(useStore.getState().previewing).toBe(true);
    vi.useRealTimers();
  });

  it("does not touch the backend when not previewing", async () => {
    vi.useFakeTimers();
    useStore.setState({ previewing: false, preview: null });
    useStore.getState().setScreen({ type: "display", id: "display:1" });
    await vi.advanceTimersByTimeAsync(500);
    expect(startPreviewMock).not.toHaveBeenCalled();
    vi.useRealTimers();
  });

  it("debounces rapid switches into a single restart with the last target", async () => {
    vi.useFakeTimers();
    useStore.setState({ previewing: true, preview: "data:old" });
    useStore.getState().setScreen({ type: "display", id: "display:1" });
    useStore.getState().setScreen({ type: "display", id: "display:2" });
    useStore.getState().setScreen({ type: "display", id: "display:3" });
    await vi.advanceTimersByTimeAsync(500);
    expect(startPreviewMock).toHaveBeenCalledTimes(1);
    const cfg = startPreviewMock.mock.calls[0][0] as unknown as { screen: { id: string } };
    expect(cfg.screen.id).toBe("display:3");
    vi.useRealTimers();
  });

  it("cancels a pending restart on stopPreview", async () => {
    vi.useFakeTimers();
    useStore.setState({ previewing: true, preview: "data:old" });
    useStore.getState().setScreen({ type: "display", id: "display:1" });
    await useStore.getState().stopPreview();
    expect(stopPreviewMock).toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(500);
    expect(startPreviewMock).not.toHaveBeenCalled();
    expect(useStore.getState().previewing).toBe(false);
    vi.useRealTimers();
  });
});

describe("startup loading phases", () => {
  it("loadBase marks booted without touching encoders", async () => {
    getDisplaysMock.mockResolvedValueOnce([{ id: "display:0", label: "Main", w: 1920, h: 1080 }]);
    useStore.setState({ encoders: [{ name: "h264_nvenc", usable: true, reason: null }] });

    await useStore.getState().loadBase();

    expect(useStore.getState().booted).toBe(true);
    expect(useStore.getState().displays).toHaveLength(1);
    // encoder probe runs separately in the background
    expect(probeEncodersMock).not.toHaveBeenCalled();
    expect(useStore.getState().encoders).toHaveLength(1);
  });

  it("loadEncoders fills encoders and clears the loading flag", async () => {
    probeEncodersMock.mockResolvedValueOnce([
      { name: "h264_nvenc", usable: true, reason: null },
    ]);

    await useStore.getState().loadEncoders();

    expect(useStore.getState().encoders).toHaveLength(1);
    expect(useStore.getState().encodersLoading).toBe(false);
  });

  it("loadEncoders falls back to empty on backend failure", async () => {
    probeEncodersMock.mockRejectedValueOnce(new Error("no backend"));

    await useStore.getState().loadEncoders();

    expect(useStore.getState().encoders).toEqual([]);
    expect(useStore.getState().encodersLoading).toBe(false);
  });

  it("loadAll runs base then encoders", async () => {
    await useStore.getState().loadAll();

    expect(getDisplaysMock).toHaveBeenCalled();
    expect(probeEncodersMock).toHaveBeenCalled();
    expect(useStore.getState().booted).toBe(true);
    expect(useStore.getState().encodersLoading).toBe(false);
  });
});
