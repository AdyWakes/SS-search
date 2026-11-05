import { useEffect, useRef, useState } from "react";
import type { Filters } from "../lib/types";
import { ChevronIcon } from "./icons";
import { shortenPath } from "../lib/format";

const DAY = 86_400_000;
const RANGES: { id: string; label: string; from: () => number | null }[] = [
  { id: "any", label: "Any time", from: () => null },
  { id: "today", label: "Today", from: () => new Date().setHours(0, 0, 0, 0) },
  { id: "week", label: "Past 7 days", from: () => Date.now() - 7 * DAY },
  { id: "month", label: "Past 30 days", from: () => Date.now() - 30 * DAY },
  { id: "year", label: "Past year", from: () => Date.now() - 365 * DAY },
];

interface Props {
  filters: Filters;
  onChange: (f: Filters) => void;
  folders: [string, number][];
  range: string;
  onRange: (id: string) => void;
}

export function rangeFrom(id: string): number | null {
  return RANGES.find((r) => r.id === id)?.from() ?? null;
}

export function FilterBar({ filters, onChange, folders, range, onRange }: Props) {
  return (
    <div className="flex flex-wrap items-center gap-1">
      <button
        type="button"
        aria-pressed={filters.screenshots_only}
        onClick={() => onChange({ ...filters, screenshots_only: !filters.screenshots_only })}
        className={`h-8 rounded-md px-3 text-[13.5px] transition-colors ${
          filters.screenshots_only ? "bg-accent text-accent-ink" : "text-ink-2 hover:bg-raised hover:text-ink"
        }`}
      >
        Screenshots only
      </button>
      <Menu
        label={RANGES.find((r) => r.id === range)?.label ?? "Any time"}
        active={range !== "any"}
        items={RANGES.map((r) => ({ id: r.id, label: r.label }))}
        value={range}
        onPick={onRange}
      />
      <Menu
        label={filters.folder ? shortenPath(filters.folder, 34) : "All folders"}
        active={!!filters.folder}
        items={[{ id: "", label: "All folders" }, ...folders.map(([p, n]) => ({ id: p, label: shortenPath(p, 52), hint: n.toLocaleString() }))]}
        value={filters.folder ?? ""}
        onPick={(id) => onChange({ ...filters, folder: id || null })}
      />
    </div>
  );
}

function Menu({
  label,
  items,
  value,
  onPick,
  active,
}: {
  label: string;
  items: { id: string; label: string; hint?: string }[];
  value: string;
  onPick: (id: string) => void;
  active: boolean;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [open]);
  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        onKeyDown={(e) => e.key === "Escape" && (e.stopPropagation(), setOpen(false))}
        className={`flex h-8 items-center gap-1 rounded-md px-3 text-[13.5px] transition-colors hover:bg-raised ${
          active ? "text-ink" : "text-ink-2 hover:text-ink"
        }`}
      >
        {label}
        <ChevronIcon size={14} />
      </button>
      {open && (
        <ul role="listbox" className="scroll absolute left-0 top-9 z-30 max-h-80 min-w-56 rounded-lg border border-line bg-raised p-1 shadow-2xl">
          {items.map((it) => (
            <li key={it.id}>
              <button
                type="button"
                role="option"
                aria-selected={it.id === value}
                onClick={() => {
                  onPick(it.id);
                  setOpen(false);
                }}
                className={`flex w-full items-center justify-between gap-6 rounded-md px-3 py-1.5 text-left text-[13.5px] hover:bg-sunken ${
                  it.id === value ? "text-accent" : "text-ink"
                }`}
              >
                <span className="truncate">{it.label}</span>
                {it.hint && <span className="text-ink-3 tabular-nums">{it.hint}</span>}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
