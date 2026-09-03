import { useTranslation } from "react-i18next";
import { useStore } from "../store";
import { MAX_AUDIO_KBPS, MAX_VIDEO_KBPS } from "../lib/types";

const BUILTIN_IDS = ["low", "mid", "high", "1080p"];

export function ProfileSelector() {
  const { t } = useTranslation();
  const profiles = useStore((s) => s.profiles);
  const profileId = useStore((s) => s.profileId);
  const setProfileId = useStore((s) => s.setProfileId);
  const encoderOverride = useStore((s) => s.encoderOverride);
  const setEncoderOverride = useStore((s) => s.setEncoderOverride);
  const encoders = useStore((s) => s.encoders);
  const encodersLoading = useStore((s) => s.encodersLoading);

  const profile = profiles?.profiles[profileId];
  const overBitrate =
    profile != null && (profile.v_kbps > MAX_VIDEO_KBPS || profile.a_kbps > MAX_AUDIO_KBPS);

  return (
    <section className="space-y-2">
      <h2 className="text-sm font-semibold text-zinc-400">{t("profile.tab")}</h2>
      <div className="flex flex-wrap gap-2 text-sm">
        {BUILTIN_IDS.map((id) => {
          const p = profiles?.profiles[id];
          if (!p) return null;
          const is1080 = id === "1080p";
          return (
            <button
              key={id}
              title={is1080 ? t("profile.warn1080p") : undefined}
              className={`rounded border px-3 py-1 ${
                profileId === id
                  ? "border-sky-500 bg-sky-900/40"
                  : "border-zinc-700 hover:border-zinc-500"
              }`}
              onClick={() => setProfileId(id)}
            >
              {t(`profile.${is1080 ? "p1080" : id}`)}
              {is1080 && <span className="ml-1 text-amber-400">⚠</span>}
            </button>
          );
        })}
        {profile && (
          <span className="self-center text-xs text-zinc-500">
            {profile.w}x{profile.h} {profile.fps}fps / {profile.v_kbps}k / {profile.a_kbps}k
          </span>
        )}
      </div>
      {profile?.warn && <p className="text-xs text-amber-500">{t("profile.warn1080p")}</p>}
      {overBitrate && <p className="text-xs font-semibold text-red-500">{t("profile.overBitrate")}</p>}

      <label className="flex items-center gap-2 text-sm">
        {t("profile.encoder")}
        <select
          className="rounded border border-zinc-700 bg-zinc-800 px-2 py-1"
          value={encoderOverride}
          onChange={(e) => setEncoderOverride(e.target.value)}
        >
          <option value="auto">{t("profile.auto")}</option>
          {encoders
            .filter((e) => e.name !== "libx264" || e.usable)
            .map((e) => (
              <option key={e.name} value={e.name} disabled={!e.usable}>
                {e.name}
                {e.reason ? ` (${e.reason})` : ""}
              </option>
            ))}
          <option value="libx264">libx264</option>
          <option value="h264_vulkan">h264_vulkan</option>
        </select>
      </label>
      {encodersLoading && (
        <p className="text-xs text-zinc-500">{t("profile.checkingEncoders")}</p>
      )}
    </section>
  );
}
