import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { useStore, type Tab } from "./store";
import { Header } from "./components/Header";
import { ScreenSelector } from "./components/ScreenSelector";
import { AudioSelector } from "./components/AudioSelector";
import { ProfileSelector } from "./components/ProfileSelector";
import { StreamControl } from "./components/StreamControl";
import { SettingsModal } from "./components/SettingsModal";
import type { PreviewFrame, StreamError } from "./lib/types";

const TABS: { id: Tab; key: string }[] = [
  { id: "screen", key: "screen.tab" },
  { id: "audio", key: "audio.tab" },
  { id: "profile", key: "profile.tab" },
];

export default function App() {
  const { t } = useTranslation();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const tab = useStore((s) => s.tab);
  const setTab = useStore((s) => s.setTab);
  const toast = useStore((s) => s.toast);
  const booted = useStore((s) => s.booted);
  const loadBase = useStore((s) => s.loadBase);
  const loadEncoders = useStore((s) => s.loadEncoders);
  const refreshStatus = useStore((s) => s.refreshStatus);

  useEffect(() => {
    // F-1: base data first (splash dismiss), encoder probe in background.
    void loadBase();
    void loadEncoders();
    const timer = setInterval(() => void refreshStatus(), 1000);
    const vuTimer = setInterval(() => void useStore.getState().refreshVu(), 100);
    // F-SC-03: live preview frames from the capture backend (1fps)
    let unlisten: (() => void) | null = null;
    void listen<PreviewFrame>("stream://preview", (e) => {
      useStore.getState().setPreview(e.payload.dataUrl);
    })
      .then((u) => {
        unlisten = u;
      })
      .catch((e) => {
        useStore.getState().showToast(String(e));
      });
    // backend failures (e.g. capture init) arrive here; without this they are silent
    let unlistenError: (() => void) | null = null;
    void listen<StreamError>("stream://error", (e) => {
      useStore.getState().showToast(`${e.payload.code}: ${e.payload.msg}`);
    })
      .then((u) => {
        unlistenError = u;
      })
      .catch(() => undefined);
    return () => {
      clearInterval(timer);
      clearInterval(vuTimer);
      unlisten?.();
      unlistenError?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="flex h-screen flex-col bg-zinc-900 text-zinc-100">
      {!booted ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-4">
          <h1 className="text-xl font-semibold">{t("app.title")}</h1>
          <div
            className="h-8 w-8 animate-spin rounded-full border-2 border-zinc-700 border-t-sky-500"
            role="status"
            aria-label={t("app.loading")}
          />
          <p className="text-sm text-zinc-400">{t("app.loading")}</p>
        </div>
      ) : (
        <>
          <Header onOpenSettings={() => setSettingsOpen(true)} />

      <nav className="flex gap-1 border-b border-zinc-800 px-4">
        {TABS.map(({ id, key }) => (
          <button
            key={id}
            className={`px-3 py-2 text-sm ${
              tab === id
                ? "border-b-2 border-sky-500 font-semibold"
                : "text-zinc-400 hover:text-zinc-200"
            }`}
            onClick={() => setTab(id)}
          >
            {t(key)}
          </button>
        ))}
      </nav>

      <main className="flex-1 space-y-4 overflow-auto p-4">
        {tab === "screen" && <ScreenSelector />}
        {tab === "audio" && <AudioSelector />}
        {tab === "profile" && <ProfileSelector />}
      </main>

      <div className="border-t border-zinc-700 p-4">
        <StreamControl />
      </div>
        </>
      )}

      {toast && (
        <div className="fixed bottom-24 left-1/2 -translate-x-1/2 rounded bg-zinc-100 px-4 py-2 text-sm font-semibold text-zinc-900 shadow-lg">
          {toast}
        </div>
      )}

      {settingsOpen && <SettingsModal onClose={() => setSettingsOpen(false)} />}
    </div>
  );
}
