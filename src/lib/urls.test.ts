import { describe, expect, it } from "vitest";
import { genericKeyWarning, playbackUrls, validateStreamKey } from "./urls";

describe("playbackUrls", () => {
  it("generates rtspt (PC) and rtsp (Quest) from the ingest URL", () => {
    const u = playbackUrls("rtmp://topaz.chat/live", "test-key-123");
    expect(u.pc).toBe("rtspt://topaz.chat/live/test-key-123");
    expect(u.quest).toBe("rtsp://topaz.chat/live/test-key-123");
  });

  it("keeps custom paths and trims trailing slash", () => {
    const u = playbackUrls("rtmp://custom.example/live/", "k");
    expect(u.pc).toBe("rtspt://custom.example/live/k");
  });
});

describe("validateStreamKey", () => {
  it("accepts 3-64 alphanumeric/-/_ keys", () => {
    expect(validateStreamKey("my-event_123")).toBeNull();
    expect(validateStreamKey("ab")).not.toBeNull();
    expect(validateStreamKey("a".repeat(65))).not.toBeNull();
    expect(validateStreamKey("bad key!")).not.toBeNull();
  });
});

describe("genericKeyWarning", () => {
  it("warns on collision-prone keys", () => {
    expect(genericKeyWarning("test")).toBe(true);
    expect(genericKeyWarning("Music")).toBe(true);
    expect(genericKeyWarning("my-event-123")).toBe(false);
  });
});
