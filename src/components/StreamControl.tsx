import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useStore } from "../store";
import { api } from "../lib/api";
import { genericKeyWarning, playbackUrls, validateStreamKey } from "../lib/urls";

export function StreamControl() {
  const { t } = useTranslation();
  const s = useStore();
  const keyError = validateStreamKey(s.streamKey);
  const genericWarn = s.streamKey.length > 0 && genericKeyWarning(s.streamKey);
  const [backendMissing, setBackendMissing] = useState(false);

  const urls = playbackUrls(s.ingestUrl, s.streamKey || "your-key");
  const canStart = !s.isLive && s.streamKey.length > 0 && keyError === null && !backendMissing;

  useEffect(() => {
    // surface the start error once (backend without capture)
    if (s.backendError?.includes("not implemented") || s.backendError?.includes("NotImplemented")) {
      setBackendMissing(true);
    }
  }, [s.backendError]);

  const copy = async (text: string) => {
    try {
      await api.copyToClipboard(text);
    } catch {
      await navigator.clipboard.writeText(text).catch(() => undefined);
    }
    s.showToast(t("stream.copied"));
  };

  return (
    <section className="space-y-2">
      <label className="flex items-center gap-2 text-sm">
        {t("stream.ingest")}
        <input
          className="flex-1 rounded border border-zinc-700 bg-zinc-800 px-2 py-1"
          value={s.ingestUrl}
          onChange={(e) => s.setIngestUrl(e.target.value)}
        />
      </label>
      <label className="block text-sm">
        {t("stream.key")}
        <input
          className={`mt-1 w-full rounded border bg-zinc-800 px-2 py-1 ${
            keyError ? "border-red-600" : "border-zinc-700"
          }`}
          value={s.streamKey}
          onChange={(e) => s.setStreamKey(e.target.value)}
          placeholder="my-event-123"
        />
        {keyError && <span className="text-xs text-red-500">{t("stream.keyInvalid")}</span>}
        {!keyError && genericWarn && (
          <span className="text-xs text-amber-500">{t("stream.keyGeneric")}</span>
        )}
      </label>

      <div className="space-y-1 text-sm">
        <CopyRow label={t("stream.copyPc")} url={urls.pc} onCopy={() => copy(urls.pc)} />
        <CopyRow label={t("stream.copyQuest")} url={urls.quest} onCopy={() => copy(urls.quest)} />
      </div>

      <button
        className={`w-full rounded py-3 text-lg font-bold ${
          s.isLive
            ? "bg-red-600 hover:bg-red-500 text-white"
            : canStart
              ? "bg-zinc-200 text-zinc-900 hover:bg-white"
              : "cursor-not-allowed bg-zinc-700 text-zinc-500"
        }`}
        disabled={!canStart && !s.isLive}
        onClick={() => (s.isLive ? void s.stopStream() : void s.startStream())}
      >
        {s.isLive ? `■ ${t("stream.stop")}` : `● ${t("stream.start")}`}
      </button>
      {backendMissing && (
        <p className="text-center text-xs text-amber-500">{t("stream.notAvailable")}</p>
      )}
    </section>
  );
}

function CopyRow({ label, url, onCopy }: { label: string; url: string; onCopy: () => void }) {
  return (
    <div className="flex items-center gap-2">
      <span className="w-20 shrink-0 text-zinc-500">{label}</span>
      <code className="flex-1 truncate rounded bg-zinc-800 px-2 py-1 text-xs">{url}</code>
      <button
        className="rounded border border-zinc-600 px-2 py-0.5 text-xs hover:border-zinc-400"
        onClick={onCopy}
      >
        copy
      </button>
    </div>
  );
}
