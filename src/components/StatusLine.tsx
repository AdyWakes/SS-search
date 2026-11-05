import type { Status } from "../lib/types";
import { formatCount, mod } from "../lib/format";

/** Thin progress line under the header while indexing. */
export function ProgressLine({ status }: { status: Status | null }) {
  if (!status) return null;
  const { phase, counts } = status;
  const active = phase === "indexing" || phase === "scanning" || phase === "starting";
  if (!active) return <div className="h-[2px]" />;
  const total = Math.max(1, counts.done + counts.pending);
  const pct = phase === "indexing" ? (counts.done / total) * 100 : null;
  return (
    <div className="relative h-[2px] overflow-hidden bg-line" role="progressbar" aria-valuenow={pct ?? undefined} aria-valuemin={0} aria-valuemax={100}>
      {pct == null ? (
        <div className="indeterminate absolute inset-y-0 w-1/3 bg-accent" />
      ) : (
        <div className="absolute inset-y-0 left-0 bg-accent transition-[width] duration-500" style={{ width: `${pct}%` }} />
      )}
    </div>
  );
}

export function StatusFooter({
  status,
  onPause,
  onResume,
  hasSelection,
  resultInfo,
}: {
  status: Status | null;
  onPause: () => void;
  onResume: () => void;
  hasSelection: boolean;
  resultInfo: string | null;
}) {
  let text = "";
  let control: "pause" | "resume" | null = null;
  if (status) {
    const c = status.counts;
    switch (status.phase) {
      case "starting":
        text = "Starting…";
        break;
      case "scanning":
        text = `Looking for images… ${formatCount(c.total)} found`;
        control = "pause";
        break;
      case "indexing": {
        const rate = status.per_minute > 0 ? ` · ${formatCount(Math.round(status.per_minute))}/min` : "";
        text = `Indexing ${formatCount(c.done)} of ${formatCount(c.done + c.pending)}${rate}`;
        control = "pause";
        break;
      }
      case "paused":
        text = `Indexing paused · ${formatCount(c.pending)} waiting`;
        control = "resume";
        break;
      case "idle":
        text = `${formatCount(c.done)} images indexed${c.failed ? ` · ${c.failed} couldn’t be read` : ""}`;
        break;
      case "stopped":
        text = status.last_error ? `Indexing stopped: ${status.last_error}` : "Indexing stopped";
        break;
    }
  }
  return (
    <footer className="flex h-10 shrink-0 items-center justify-between gap-4 border-t border-line px-5 text-[13px] text-ink-2">
      <div className="flex min-w-0 items-center gap-3">
        <span className="truncate">{text}</span>
        {control && (
          <button type="button" onClick={control === "pause" ? onPause : onResume} className="rounded px-1.5 text-ink hover:bg-raised">
            {control === "pause" ? "Pause" : "Resume"}
          </button>
        )}
        {resultInfo && <span className="hidden text-ink-3 md:inline">{resultInfo}</span>}
      </div>
      {hasSelection && (
        <div className="hidden shrink-0 items-center gap-4 lg:flex">
          <Hint k="Enter" label="Open" />
          <Hint k="Space" label="Preview" />
          <Hint k={`${mod}+Enter`} label="Show in folder" />
          <Hint k={`${mod}+C`} label="Copy image" />
          <Hint k={`${mod}+Shift+C`} label="Copy text" />
        </div>
      )}
    </footer>
  );
}

function Hint({ k, label }: { k: string; label: string }) {
  return (
    <span className="flex items-center gap-1.5">
      <kbd>{k}</kbd>
      <span>{label}</span>
    </span>
  );
}
