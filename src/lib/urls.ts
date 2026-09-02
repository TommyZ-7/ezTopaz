// Playback URL generation (F-URL-01/02): rtmp ingest → rtspt (PC) / rtsp (Quest).
export function playbackUrls(ingestUrl: string, key: string): { pc: string; quest: string } {
  const base = ingestUrl.replace(/^rtmp:\/\//, "").replace(/\/+$/, "");
  return { pc: `rtspt://${base}/${key}`, quest: `rtsp://${base}/${key}` };
}

/** F-ST-01: 3-64 chars, alphanumeric / hyphen / underscore. */
export function validateStreamKey(key: string): string | null {
  const ok = key.length >= 3 && key.length <= 64 && /^[A-Za-z0-9_-]+$/.test(key);
  return ok ? null : "stream.keyInvalid";
}

/** Generic keys risk colliding with other people's streams (requirements §2.3). */
const GENERIC_KEYS = ["test", "music", "live", "stream", "key", "vrchat"];
export function genericKeyWarning(key: string): boolean {
  return GENERIC_KEYS.includes(key.toLowerCase());
}
