import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useStore } from "../store";
import { api } from "../lib/api";
import { MAX_AUDIO_KBPS, MAX_VIDEO_KBPS, type ProfilesConfig } from "../lib/types";

export function SettingsModal({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const profiles = useStore((s) => s.profiles);
  const setProfiles = useStore((s) => s.setProfiles);
  const showToast = useStore((s) => s.showToast);
  const [dirty, setDirty] = useState<ProfilesConfig | null>(null);
  const cfg = dirty ?? profiles;

  const update = (fn: (draft: ProfilesConfig) => void) => {
    if (!cfg) return;
    const next = structuredClone(cfg);
    fn(next);
    setDirty(next);
  };

  const save = async () => {
    if (!dirty) return;
    try {
      await api.saveProfiles(dirty);
      setProfiles(dirty);
      setDirty(null);
      showToast(t("app.saved"));
    } catch (e) {
      showToast(String(e));
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
      <div className="max-h-[85vh] w-[560px] overflow-auto rounded-lg border border-zinc-700 bg-zinc-900 p-4">
        <div className="mb-3 flex items-center">
          <h2 className="font-bold">{t("settings.title")}</h2>
          <button className="ml-auto text-zinc-400 hover:text-white" onClick={onClose}>
            ✕
          </button>
        </div>

        <h3 className="mb-1 text-sm font-semibold text-zinc-400">{t("settings.profiles")}</h3>
        <table className="w-full text-sm">
          <thead className="text-left text-xs text-zinc-500">
            <tr>
              <th className="pr-2">{t("settings.name")}</th>
              <th className="pr-2">{t("settings.resolution")}</th>
              <th className="pr-2">{t("settings.fps")}</th>
              <th className="pr-2">{t("settings.vKbps")}</th>
              <th className="pr-2">{t("settings.aKbps")}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {Object.entries(cfg?.profiles ?? {}).map(([id, p]) => (
              <tr key={id} className="border-t border-zinc-800">
                <td className="py-1 pr-2">{t(p.name.replace(/^profile\./, "profile."))}</td>
                <td className="pr-2 text-zinc-400">
                  {p.w}x{p.h}
                </td>
                <td className="pr-2 text-zinc-400">{p.fps}</td>
                <td className="pr-2">
                  <NumberCell
                    value={p.v_kbps}
                    max={MAX_VIDEO_KBPS}
                    onChange={(v) => update((d) => void (d.profiles[id].v_kbps = v))}
                  />
                </td>
                <td className="pr-2">
                  <NumberCell
                    value={p.a_kbps}
                    max={MAX_AUDIO_KBPS}
                    onChange={(v) => update((d) => void (d.profiles[id].a_kbps = v))}
                  />
                </td>
                <td className="text-right">
                  <button
                    className="mr-1 text-xs text-zinc-400 hover:text-white"
                    onClick={() =>
                      update((d) => {
                        let n = 1;
                        while (d.profiles[`${id}-copy${n}`]) n++;
                        d.profiles[`${id}-copy${n}`] = { ...p, name: `${p.name}-copy${n}` };
                      })
                    }
                  >
                    {t("settings.duplicate")}
                  </button>
                  <button
                    className="text-xs text-zinc-400 hover:text-red-400 disabled:opacity-30"
                    disabled={BUILTIN_IDS.includes(id)}
                    onClick={() => update((d) => void delete d.profiles[id])}
                  >
                    {t("settings.delete")}
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {dirty && (
          <div className="mt-2 flex justify-end">
            <button
              className="rounded bg-sky-600 px-3 py-1 text-sm font-semibold hover:bg-sky-500"
              onClick={() => void save()}
            >
              {t("app.save")}
            </button>
          </div>
        )}

        <div className="mt-4 border-t border-zinc-800 pt-3 text-sm">
          <button
            className="text-sky-400 hover:underline"
            onClick={() => void api.openLogsDir().catch(() => undefined)}
          >
            {t("app.logs")}
          </button>
          <p className="mt-2 text-xs text-zinc-500">{t("app.licenses")}</p>
          <p className="mt-1 text-xs text-zinc-600">
            ezTopaz: MIT — FFmpeg: LGPL 2.1+ / GPL (libx264). Sources:
            https://ffmpeg.org/download.html
          </p>
        </div>
      </div>
    </div>
  );
}

const BUILTIN_IDS = ["low", "mid", "high", "1080p"];

function NumberCell({ value, max, onChange }: { value: number; max: number; onChange: (v: number) => void }) {
  const over = value > max;
  return (
    <input
      type="number"
      className={`w-20 rounded border bg-zinc-800 px-1 py-0.5 ${over ? "border-red-600" : "border-zinc-700"}`}
      value={value}
      onChange={(e) => onChange(Number(e.target.value))}
    />
  );
}
