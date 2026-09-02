import { useTranslation } from "react-i18next";
import { useStore } from "../store";
import { api } from "../lib/api";

function VuBar({ level }: { level: number }) {
  const pct = Math.min(100, Math.round(level * 100));
  return (
    <div className="h-2 w-20 overflow-hidden rounded bg-zinc-700">
      <div
        className={`h-full ${pct > 90 ? "bg-red-500" : "bg-emerald-500"}`}
        style={{ width: `${pct}%` }}
      />
    </div>
  );
}

export function AudioSelector() {
  const { t } = useTranslation();
  const { audioMode, setAudioMode, audioDevices, selectedApps, toggleApp, mic, setMic, vu, isLive } =
    useStore((s) => ({
      audioMode: s.audioMode,
      setAudioMode: s.setAudioMode,
      audioDevices: s.audioDevices,
      selectedApps: s.selectedApps,
      toggleApp: s.toggleApp,
      mic: s.mic,
      setMic: s.setMic,
      vu: s.vu,
      isLive: s.isLive,
    }));

  // F-AU-03/04: live gain/mute changes reach the running mixer
  const pushMix = () => {
    const st = useStore.getState();
    if (!st.isLive) return;
    void api
      .updateAudioMix({
        apps: Object.fromEntries(
          st.selectedApps.map((a) => [a, { gain: st.mic.gain, muted: false }])
        ),
        mic: { enabled: st.mic.enabled, muted: st.mic.muted, gain: st.mic.gain },
      })
      .catch(() => undefined);
  };

  return (
    <section className="space-y-2">
      <h2 className="text-sm font-semibold text-zinc-400">{t("audio.tab")}</h2>
      <div className="flex gap-4 text-sm">
        <label className="flex items-center gap-1">
          <input
            type="radio"
            checked={audioMode === "system"}
            onChange={() => setAudioMode("system")}
          />
          {t("audio.system")}
        </label>
        <label className="flex items-center gap-1">
          <input type="radio" checked={audioMode === "apps"} onChange={() => setAudioMode("apps")} />
          {t("audio.apps")}
        </label>
      </div>

      {audioMode === "apps" && (
        <div className="flex flex-wrap gap-3 text-sm">
          {(audioDevices?.apps ?? []).map((a) => (
            <label key={a.id} className="flex items-center gap-1">
              <input
                type="checkbox"
                checked={selectedApps.includes(a.id)}
                onChange={() => toggleApp(a.id)}
              />
              {a.label}
              {isLive && selectedApps.includes(a.id) && (
                <VuBar level={vu.apps[a.id]?.rms ?? 0} />
              )}
            </label>
          ))}
          {audioDevices === null && (
            <p className="text-xs text-zinc-500">{t("screen.notAvailable")}</p>
          )}
        </div>
      )}

      <div className="flex items-center gap-3 text-sm">
        <label className="flex items-center gap-1">
          <input
            type="checkbox"
            checked={mic.enabled}
            onChange={(e) => setMic({ enabled: e.target.checked })}
          />
          {t("audio.mic")}
        </label>
        <select
          className="rounded border border-zinc-700 bg-zinc-800 px-2 py-1"
          value={mic.device}
          onChange={(e) => setMic({ device: e.target.value })}
        >
          <option value="default">default</option>
          {(audioDevices?.inputs ?? []).map((d) => (
            <option key={d.id} value={d.id}>
              {d.label}
            </option>
          ))}
        </select>
        <label className="flex items-center gap-1">
          <input
            type="checkbox"
            checked={mic.muted}
            onChange={(e) => setMic({ muted: e.target.checked })}
          />
          {t("audio.mute")}
        </label>
        <input
          type="range"
          min={0}
          max={2}
          step={0.05}
          value={mic.gain}
          onChange={(e) => setMic({ gain: Number(e.target.value) })}
          onMouseUp={pushMix}
          onTouchEnd={pushMix}
          title={t("audio.gain")}
          className="w-24"
        />
        {isLive && mic.enabled && <VuBar level={vu.mic?.rms ?? 0} />}
      </div>
    </section>
  );
}
