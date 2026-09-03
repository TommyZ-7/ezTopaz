import { useTranslation } from "react-i18next";
import { useStore } from "../store";
import { api } from "../lib/api";

export function ScreenSelector() {
  const { t } = useTranslation();
  const displays = useStore((s) => s.displays);
  const windows = useStore((s) => s.windows);
  const screen = useStore((s) => s.screen);
  const setScreen = useStore((s) => s.setScreen);
  const backendError = useStore((s) => s.backendError);
  const preview = useStore((s) => s.preview);
  const startPortalPicker = async () => {
    try {
      const target = await api.startPortalPicker();
      useStore.getState().setScreen(target);
    } catch (e) {
      // Surface portal failures (e.g. no D-Bus session under sudo, user
      // cancelled, compositor rejected): silent catch here once made the
      // button look dead with no clue why.
      useStore.getState().showToast(String(e));
    }
  };

  return (
    <section className="space-y-2">
      <h2 className="text-sm font-semibold text-zinc-400">{t("screen.tab")}</h2>
      <div className="flex gap-4 text-sm">
        <label className="flex items-center gap-1">
          <input
            type="radio"
            checked={screen.type === "display"}
            onChange={() => setScreen({ type: "display", id: displays[0]?.id ?? screen.id })}
          />
          {t("screen.display")}
        </label>
        <label className="flex items-center gap-1">
          <input
            type="radio"
            checked={screen.type === "window"}
            onChange={() => setScreen({ type: "window", id: windows[0]?.id ?? screen.id })}
          />
          {t("screen.window")}
        </label>
      </div>

      {screen.type === "display" && (
        <div className="flex flex-wrap gap-2 text-sm">
          {displays.map((d) => (
            <button
              key={d.id}
              className={`rounded border px-3 py-1 ${
                screen.id === d.id
                  ? "border-sky-500 bg-sky-900/40"
                  : "border-zinc-700 hover:border-zinc-500"
              }`}
              onClick={() => setScreen({ type: "display", id: d.id })}
            >
              {d.label} ({d.w}x{d.h})
            </button>
          ))}
          {displays.length === 0 && (
            <p className="text-zinc-500 text-xs">{t("screen.notAvailable")}</p>
          )}
        </div>
      )}

      {screen.type === "window" && (
        <div className="space-y-2 text-sm">
          {windows.length === 0 ? (
            <button
              className="rounded border border-zinc-600 px-3 py-1 hover:border-zinc-400"
              onClick={() => void startPortalPicker()}
            >
              {t("screen.portalPicker")}
            </button>
          ) : (
            <select
              className="w-full rounded border border-zinc-700 bg-zinc-800 px-2 py-1"
              value={screen.id}
              onChange={(e) => setScreen({ type: "window", id: e.target.value })}
            >
              <option value="">--</option>
              {windows.map((w) => (
                <option key={w.id} value={w.id}>
                  {w.title} ({w.app})
                </option>
              ))}
            </select>
          )}
        </div>
      )}

      {backendError && (
        <p className="text-xs text-amber-500">{t("screen.notAvailable")}</p>
      )}

      {/* F-SC-03 preview */}
      {preview ? (
        <img
          src={preview}
          alt="preview"
          className="aspect-video w-full rounded border border-zinc-700 bg-black object-contain"
        />
      ) : (
        <div className="flex aspect-video w-full items-center justify-center rounded border border-zinc-700 bg-zinc-800 text-xs text-zinc-600">
          preview 16:9
        </div>
      )}
    </section>
  );
}
