import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useStore, type Tab } from "./store";
import { Header } from "./components/Header";
import { ScreenSelector } from "./components/ScreenSelector";
import { AudioSelector } from "./components/AudioSelector";
import { ProfileSelector } from "./components/ProfileSelector";
import { StreamControl } from "./components/StreamControl";
import { SettingsModal } from "./components/SettingsModal";

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
  const loadAll = useStore((s) => s.loadAll);
  const refreshStatus = useStore((s) => s.refreshStatus);

  useEffect(() => {
    void loadAll();
    const timer = setInterval(() => void refreshStatus(), 1000);
    return () => clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="flex h-screen flex-col bg-zinc-900 text-zinc-100">
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

      {toast && (
        <div className="fixed bottom-24 left-1/2 -translate-x-1/2 rounded bg-zinc-100 px-4 py-2 text-sm font-semibold text-zinc-900 shadow-lg">
          {toast}
        </div>
      )}

      {settingsOpen && <SettingsModal onClose={() => setSettingsOpen(false)} />}
    </div>
  );
}
