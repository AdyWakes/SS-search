import { useEffect, useState } from "react";
import type { DownloadProgress, ModelId, ModelsView } from "../lib/types";
import { useApi } from "../lib/ApiContext";
import { formatBytes } from "../lib/format";
import { CheckIcon, ShieldIcon } from "./icons";

/** First run: explain the one-time download and ask before any network use. */
export function Onboarding({ view, onDone }: { view: ModelsView; onDone: () => void }) {
  const api = useApi();
  const [chosen, setChosen] = useState<Set<ModelId>>(
    () => new Set(view.models.filter((m) => m.pinned && !m.installed).map((m) => m.id)),
  );
  const [progress, setProgress] = useState<DownloadProgress | null>(null);
  const [busy, setBusy] = useState(view.downloading);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const subs = [
      api.onModelProgress((p) => setProgress(p)),
      api.onModelsChanged((e) => {
        setBusy(false);
        if (e) setError(e);
        else onDone();
      }),
    ];
    return () => subs.forEach((s) => s.then((u) => u()));
  }, [api, onDone]);

  const selected = view.models.filter((m) => chosen.has(m.id));
  const total = selected.reduce((n, m) => n + m.download_bytes, 0);
  const toggle = (id: ModelId) =>
    setChosen((s) => {
      const n = new Set(s);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });

  const start = async () => {
    setError(null);
    setBusy(true);
    try {
      await api.downloadModels([...chosen]);
    } catch (e) {
      setBusy(false);
      setError(String(e));
    }
  };
  const importDir = async () => {
    const dir = await api.pickFolder();
    if (!dir) return;
    try {
      await api.importModels(dir);
      onDone();
    } catch (e) {
      setError(String(e));
    }
  };
  const skip = async () => {
    await api.skipModels();
    onDone();
  };

  const pct = progress && progress.overall_total ? Math.min(100, (progress.overall_downloaded / progress.overall_total) * 100) : 0;
  return (
    <div className="scroll flex h-full justify-center px-8">
      <div className="w-full max-w-[640px] py-16">
        <div className="mb-6 flex h-11 w-11 items-center justify-center rounded-xl bg-raised text-accent">
          <ShieldIcon size={24} />
        </div>
        <h1 className="font-display text-[34px] font-semibold leading-[1.15] tracking-[-0.01em]">
          One download. Then nothing leaves your computer.
        </h1>
        <p className="mt-4 max-w-[560px] text-[15px] leading-[1.6] text-ink-2">
          shotfind reads the text in your screenshots and recognises what they show using two small models that run on this computer. They are
          downloaded once from the sources below and checked against fixed SHA-256 fingerprints. After that, shotfind makes no network
          connections and sends no telemetry.
        </p>

        <ul className="mt-8 divide-y divide-line border-y border-line">
          {view.models.map((m) => {
            const disabled = m.installed || !m.pinned || busy;
            return (
              <li key={m.id}>
                <label className={`flex gap-4 py-4 ${disabled ? "" : "cursor-pointer"}`}>
                  <input
                    type="checkbox"
                    className="mt-1 h-4 w-4 accent-[var(--accent)]"
                    checked={m.installed || chosen.has(m.id)}
                    disabled={disabled || m.required}
                    onChange={() => toggle(m.id)}
                  />
                  <div className="min-w-0 flex-1">
                    <div className="flex items-baseline justify-between gap-4">
                      <span className="text-[15px] font-semibold">{m.title}</span>
                      <span className="shrink-0 text-[14px] tabular-nums text-ink-2">
                        {m.installed ? (
                          <span className="inline-flex items-center gap-1 text-ink">
                            <CheckIcon size={15} /> Installed
                          </span>
                        ) : (
                          formatBytes(m.download_bytes)
                        )}
                      </span>
                    </div>
                    <p className="mt-1 text-[14px] leading-[1.5] text-ink-2">{m.detail}</p>
                    <p className="mt-1 text-[13px] text-ink-3">
                      From {m.sources.join(", ")} · {m.license}
                      {m.required ? " · needed for text search" : " · optional"}
                    </p>
                    {!m.pinned && (
                      <p className="mt-1 text-[13px] text-accent">
                        This build has no pinned fingerprint for this model, so it won’t be downloaded.
                      </p>
                    )}
                  </div>
                </label>
              </li>
            );
          })}
        </ul>

        {busy ? (
          <div className="mt-8">
            <div className="flex justify-between text-[14px]">
              <span>{progress?.stage === "extracting" ? "Verifying and unpacking…" : "Downloading…"}</span>
              <span className="tabular-nums text-ink-2">
                {progress ? `${formatBytes(progress.overall_downloaded)} of ${formatBytes(progress.overall_total)}` : ""}
              </span>
            </div>
            <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-raised">
              <div className="h-full rounded-full bg-accent transition-[width] duration-200" style={{ width: `${pct}%` }} />
            </div>
            <button type="button" onClick={() => api.cancelDownload()} className="mt-4 text-[14px] text-ink-2 hover:text-ink">
              Cancel
            </button>
          </div>
        ) : (
          <div className="mt-8 flex flex-wrap items-center gap-3">
            <button
              type="button"
              onClick={start}
              disabled={chosen.size === 0}
              className="h-11 rounded-lg bg-accent px-6 text-[15px] font-semibold text-accent-ink transition hover:brightness-110 disabled:opacity-40"
            >
              {chosen.size === 0 ? "Nothing to download" : `Download ${formatBytes(total)}`}
            </button>
            <button type="button" onClick={skip} className="h-11 rounded-lg px-4 text-[15px] text-ink-2 hover:bg-raised hover:text-ink">
              Not now · search file names only
            </button>
          </div>
        )}
        {error && <p className="mt-4 text-[14px] leading-[1.5] text-accent">{error}</p>}
        {!busy && (
          <button type="button" onClick={importDir} className="mt-6 text-[13.5px] text-ink-2 underline decoration-line underline-offset-4 hover:text-ink">
            Offline? Install from a folder that already has the model files
          </button>
        )}
      </div>
    </div>
  );
}
