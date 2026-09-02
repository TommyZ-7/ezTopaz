import { useTranslation } from "react-i18next";
import { useStore } from "../store";

export function AudioSelector() {
  const { t } = useTranslation();
  const { audioMode, setAudioMode, audioDevices, selectedApps, toggleApp, mic, setMic } =
    useStore((s) => ({
      audioMode: s.audioMode,
      setAudioMode: s.setAudioMode,
      audioDevices: s.audioDevices,
      selectedApps: s.selectedApps,
      toggleApp: s.toggleApp,
      mic: s.mic,
      setMic: s.setMic,
    }));

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
          title={t("audio.gain")}
          className="w-24"
        />
      </div>
    </section>
  );
}
