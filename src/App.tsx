import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getApi, type Api } from "./lib/api";
import { ApiContext, useApi } from "./lib/ApiContext";
import type { Filters, ModelsView, Status } from "./lib/types";
import { useSearch } from "./lib/useSearch";
import { handleResultKeys, useActions } from "./lib/useActions";
import { ToastProvider } from "./components/Toast";
import { ResultGrid, gridColumns } from "./components/ResultGrid";
import { FilterBar, rangeFrom } from "./components/FilterBar";
import { ProgressLine, StatusFooter } from "./components/StatusLine";
import { Preview } from "./components/Preview";
import { SettingsPage } from "./components/Settings";
import { Onboarding } from "./components/Onboarding";
import { QuickBar } from "./components/QuickBar";
import { BackIcon, GearIcon, SearchIcon } from "./components/icons";
import { formatCount } from "./lib/format";

const isBar = new URLSearchParams(location.search).get("view") === "bar";

export default function App() {
  const [api, setApi] = useState<Api | null>(null);
  useEffect(() => {
    getApi().then(setApi);
  }, []);
  if (!api) return null;
  return (
    <ApiContext.Provider value={api}>
      <ToastProvider>{isBar ? <QuickBar /> : <Main />}</ToastProvider>
    </ApiContext.Provider>
  );
}

function Main() {
  const api = useApi();
  const act = useActions();
  const [status, setStatus] = useState<Status | null>(null);
  const [models, setModels] = useState<ModelsView | null>(null);
  const [page, setPage] = useState<"search" | "settings" | "models">("search");
  const [text, setText] = useState("");
  const [filters, setFilters] = useState<Filters>({ screenshots_only: false });
  const [range, setRange] = useState("any");
  const [folders, setFolders] = useState<[string, number][]>([]);
  const [sel, setSel] = useState(0);
  const [preview, setPreview] = useState<number | null>(null);
  const [refresh, setRefresh] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const body = useRef<HTMLDivElement>(null);

  const loadModels = useCallback(() => api.models().then(setModels), [api]);
  useEffect(() => {
    api.status().then(setStatus);
    loadModels();
    api.foldersInIndex().then(setFolders);
    const subs = [
      api.onStatus(setStatus),
      api.onSetQuery((q) => {
        setPage("search");
        setText(q);
        input.current?.focus();
      }),
      api.onModelsChanged(() => loadModels()),
    ];
    return () => subs.forEach((s) => s.then((u) => u()));
  }, [api, loadModels]);

  // Refresh results (and folder list) as indexing makes progress.
  const done = status?.counts.done ?? 0;
  const bucket = done < 200 ? Math.floor(done / 20) : Math.floor(done / 250);
  useEffect(() => {
    setRefresh((r) => r + 1);
    api.foldersInIndex().then(setFolders);
  }, [api, bucket, status?.phase]);

  const effFilters = useMemo(() => ({ ...filters, from_ms: rangeFrom(range) }), [filters, range]);
  const { results, error } = useSearch(api, text, effFilters, 120, refresh);
  const hits = results?.hits ?? [];
  useEffect(() => setSel(0), [text, filters, range]);

  const needsOnboarding = models && !models.consented && models.models.some((m) => !m.installed);
  const showModels = page === "models" || needsOnboarding;

  const onKey = (e: React.KeyboardEvent) => {
    if (page !== "search" || showModels) return;
    if (preview != null) {
      if (e.key === "Escape" || e.key === " ") {
        e.preventDefault();
        setPreview(null);
      } else if (handleResultKeys(e, hits[preview]?.id ?? null, act)) e.preventDefault();
      return;
    }
    if (e.key === "Escape") {
      if (text) setText("");
      return;
    }
    const cols = gridColumns(body.current);
    const move = { ArrowRight: 1, ArrowLeft: -1, ArrowDown: cols, ArrowUp: -cols }[e.key];
    if (move && hits.length) {
      // Left/right only move the selection when the caret isn't editing text.
      const editing = document.activeElement === input.current && (e.key === "ArrowLeft" || e.key === "ArrowRight") && text.length > 0;
      if (!editing) {
        e.preventDefault();
        setSel((s) => Math.min(hits.length - 1, Math.max(0, s + move)));
        return;
      }
    }
    if (e.key === " " && document.activeElement !== input.current && hits[sel]) {
      e.preventDefault();
      setPreview(sel);
      return;
    }
    if (handleResultKeys(e, hits[sel]?.id ?? null, act)) e.preventDefault();
  };

  const resultInfo = results && text.trim() ? `${formatCount(hits.length)} ${hits.length === 1 ? "result" : "results"} · ${Math.round(results.took_ms)} ms` : null;
  const noModels = status && !status.ocr_available;

  return (
    <div className="flex h-full flex-col" onKeyDown={onKey}>
      <header className="shrink-0 px-5 pt-4">
        <div className="flex items-center gap-3">
          {page !== "search" || showModels ? (
            <div className="flex h-[60px] flex-1 items-center gap-2">
              {!needsOnboarding && (
                <button type="button" onClick={() => setPage("search")} aria-label="Back to search" className="-ml-1 rounded-md p-1.5 text-ink-2 hover:bg-raised hover:text-ink">
                  <BackIcon size={22} />
                </button>
              )}
              {!needsOnboarding && <h1 className="font-display text-[22px] font-semibold">{page === "models" ? "Search models" : "Settings"}</h1>}
            </div>
          ) : (
            <label className="flex h-[60px] flex-1 items-center gap-4 rounded-xl bg-raised px-5 ring-1 ring-line focus-within:ring-2 focus-within:ring-accent">
              <SearchIcon size={24} className="shrink-0 text-ink-2" />
              <input
                ref={input}
                value={text}
                onChange={(e) => setText(e.target.value)}
                placeholder="Type what you remember…"
                aria-label="Search screenshots"
                spellCheck={false}
                autoFocus
                className="h-full min-w-0 flex-1 bg-transparent font-display text-[24px] text-ink outline-none placeholder:text-ink-3"
              />
            </label>
          )}
          {!needsOnboarding && page === "search" && (
            <button type="button" onClick={() => setPage("settings")} aria-label="Settings" className="rounded-lg p-2.5 text-ink-2 hover:bg-raised hover:text-ink">
              <GearIcon size={22} />
            </button>
          )}
        </div>
        {page === "search" && !showModels && (
          <div className="mt-2 flex items-center justify-between">
            <FilterBar filters={filters} onChange={setFilters} folders={folders} range={range} onRange={setRange} />
          </div>
        )}
      </header>
      <div className="mt-3">
        <ProgressLine status={status} />
      </div>

      <main ref={body} className="min-h-0 flex-1">
        {showModels && models ? (
          <Onboarding
            view={models}
            onDone={() => {
              loadModels();
              setPage("search");
            }}
          />
        ) : page === "settings" ? (
          <SettingsPage status={status} onModels={() => setPage("models")} />
        ) : (
          <div className="scroll h-full px-5 pb-8 pt-5">
            {noModels && models?.consented && (
              <div className="mb-5 flex items-center justify-between gap-4 rounded-lg bg-raised px-4 py-3 text-[14px]">
                <span>Searching file names only. Add the text and meaning models to search inside your images.</span>
                <button type="button" onClick={() => setPage("models")} className="shrink-0 font-semibold text-accent hover:underline">
                  Set up
                </button>
              </div>
            )}
            {results?.corrections.length ? (
              <p className="mb-4 text-[14px] text-ink-2">
                Also matching{" "}
                {results.corrections.map(([, to], i) => (
                  <span key={to}>
                    {i > 0 && ", "}
                    <span className="text-ink">{to}</span>
                  </span>
                ))}
              </p>
            ) : null}
            {!text.trim() && hits.length > 0 && <h2 className="mb-4 font-display text-[15px] font-semibold text-ink-2">Recent</h2>}
            {error && <p className="text-[14px] text-accent">{error}</p>}
            {hits.length > 0 ? (
              <ResultGrid hits={hits} query={[text, ...(results?.corrections ?? []).map((c) => c[1])].join(" ")} selected={sel} onSelect={setSel} onOpen={(i) => act.open(hits[i].id)} onPreview={setPreview} />
            ) : results ? (
              <Empty text={text} status={status} />
            ) : null}
          </div>
        )}
      </main>

      {page === "search" && !showModels && (
        <StatusFooter
          status={status}
          onPause={() => api.pause()}
          onResume={() => api.resume()}
          hasSelection={hits.length > 0}
          resultInfo={resultInfo}
        />
      )}
      {preview != null && hits[preview] && <Preview hit={hits[preview]} onClose={() => setPreview(null)} />}
    </div>
  );
}

function Empty({ text, status }: { text: string; status: Status | null }) {
  const indexing = status && (status.phase === "indexing" || status.phase === "scanning");
  return (
    <div className="mx-auto mt-24 max-w-[440px] text-center">
      <p className="font-display text-[20px] font-semibold">{text.trim() ? "No matches" : "No images yet"}</p>
      <p className="mt-2 text-[14.5px] leading-[1.55] text-ink-2">
        {text.trim()
          ? indexing
            ? "Indexing is still running, so more results may appear. Try fewer or different words."
            : "Try fewer words, or describe what the image shows instead of the text in it."
          : "Add folders in Settings, or take a screenshot. New images appear here within seconds."}
      </p>
    </div>
  );
}
