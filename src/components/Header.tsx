import { useTranslation } from "react-i18next";
import { useLocale } from "../locale";
import { useStore } from "../store";

export function Header({ onOpenSettings }: { onOpenSettings: () => void }) {
  const { t } = useTranslation();
  const isLive = useStore((s) => s.isLive);
  const status = useStore((s) => s.status);
  const locale = useLocale((s) => s.locale);
  const setLocale = useLocale((s) => s.setLocale);

  const mm = String(Math.floor(status.durationSec / 60)).padStart(2, "0");
  const ss = String(status.durationSec % 60).padStart(2, "0");

  return (
    <header className="flex items-center gap-3 border-b border-zinc-700 px-4 py-2">
      <h1 className="font-bold">{t("app.title")}</h1>
      {isLive ? (
        <span className="flex items-center gap-1 text-red-400 text-sm">
          <span className="inline-block h-2 w-2 rounded-full bg-red-500 animate-pulse" />
          {t("stream.live")} {mm}:{ss}
          {status.retrying != null && (
            <span className="text-amber-400">
              {t("stream.retrying", { n: status.retrying })}
            </span>
          )}
        </span>
      ) : (
        <span className="text-zinc-500 text-sm">{t("stream.stopped")}</span>
      )}
      <div className="ml-auto flex items-center gap-2">
        <button
          className="rounded px-2 py-1 text-sm hover:bg-zinc-700"
          onClick={() => setLocale(locale === "ja" ? "en" : "ja")}
          aria-label="language"
        >
          {locale === "ja" ? "ja/en" : "en/ja"}
        </button>
        <button
          className="rounded px-2 py-1 text-sm hover:bg-zinc-700"
          onClick={onOpenSettings}
          aria-label={t("app.settings")}
        >
          ⚙
        </button>
      </div>
    </header>
  );
}
