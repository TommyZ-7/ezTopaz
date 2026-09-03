import { beforeEach, describe, expect, it, vi } from "vitest";

const startPreviewMock = vi.fn(async (_cfg: unknown) => undefined);
const stopPreviewMock = vi.fn(async () => undefined);

vi.mock("./lib/api", () => ({
  api: {
    startPreview: (cfg: unknown) => startPreviewMock(cfg),
    stopPreview: () => stopPreviewMock(),
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
