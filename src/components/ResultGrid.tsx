import { memo, useEffect, useRef } from "react";
import type { Hit } from "../lib/types";
import { useApi } from "../lib/ApiContext";
import { focusSnippet, formatDate, highlight } from "../lib/format";

interface Props {
  hits: Hit[];
  query: string;
  selected: number;
  onSelect: (i: number) => void;
  onOpen: (i: number) => void;
  onPreview: (i: number) => void;
  compact?: boolean;
}

const KIND_LABEL: Record<Hit["kind"], string | null> = {
  recent: null,
  text: null,
  name: "File name",
  meaning: "Looks like a match",
  both: null,
};

export function ResultGrid({ hits, query, selected, onSelect, onOpen, onPreview, compact }: Props) {
  const gridRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = gridRef.current?.querySelector<HTMLElement>(`[data-index="${selected}"]`);
    el?.scrollIntoView({ block: "nearest" });
  }, [selected]);
  return (
    <div
      ref={gridRef}
      role="listbox"
      aria-label="Results"
      className={`grid ${compact ? "gap-x-3 gap-y-3" : "gap-x-5 gap-y-6"}`}
      style={{ gridTemplateColumns: `repeat(auto-fill, minmax(${compact ? 160 : 224}px, 1fr))` }}
    >
      {hits.map((h, i) => (
        <Tile
          key={h.id}
          hit={h}
          index={i}
          query={query}
          selected={i === selected}
          compact={compact}
          onSelect={onSelect}
          onOpen={onOpen}
          onPreview={onPreview}
        />
      ))}
    </div>
  );
}

/** Number of columns currently rendered by the grid (for arrow-key navigation). */
export function gridColumns(container: HTMLElement | null): number {
  const grid = container?.querySelector<HTMLElement>('[role="listbox"]');
  if (!grid) return 1;
  return Math.max(1, getComputedStyle(grid).gridTemplateColumns.split(" ").filter(Boolean).length);
}

const Tile = memo(function Tile({
  hit,
  index,
  query,
  selected,
  compact,
  onSelect,
  onOpen,
  onPreview,
}: {
  hit: Hit;
  index: number;
  query: string;
  selected: boolean;
  compact?: boolean;
  onSelect: (i: number) => void;
  onOpen: (i: number) => void;
  onPreview: (i: number) => void;
}) {
  const api = useApi();
  const label = KIND_LABEL[hit.kind];
  const caption = hit.snippet ? focusSnippet(hit.snippet, query) : label ? null : formatDate(hit.mtime_ms);
  return (
    <div
      role="option"
      aria-selected={selected}
      data-index={index}
      title={hit.path}
      onMouseDown={() => onSelect(index)}
      onDoubleClick={() => onOpen(index)}
      onContextMenu={(e) => {
        e.preventDefault();
        onSelect(index);
        onPreview(index);
      }}
      className="group min-w-0 cursor-default outline-none"
    >
      <div
        className={`relative aspect-[16/10] overflow-hidden rounded-[10px] bg-sunken transition-shadow ${
          selected ? "ring-2 ring-accent ring-offset-2 ring-offset-bg" : "ring-1 ring-line group-hover:ring-ink-3"
        }`}
      >
        <img
          src={api.thumbUrl(hit.id)}
          alt=""
          loading="lazy"
          decoding="async"
          draggable={false}
          className={`h-full w-full ${(hit.height ?? 0) > (hit.width ?? 0) * 1.2 ? "object-contain" : "object-cover object-top"}`}
        />
      </div>
      {compact && <div className="mt-1.5 truncate px-0.5 text-[12.5px] leading-4 text-ink-2">{hit.name}</div>}
      {!compact && (
        <div className="mt-2 px-0.5">
          <div className={`truncate text-[13.5px] leading-5 ${selected ? "text-ink" : "text-ink"}`}>{hit.name}</div>
          <div className="truncate text-[13px] leading-5 text-ink-2">
            {caption ? <Highlighted text={caption} query={hit.snippet ? query : ""} /> : label}
          </div>
        </div>
      )}
    </div>
  );
});

function Highlighted({ text, query }: { text: string; query: string }) {
  return (
    <>
      {highlight(text, query).map((p, i) => (p.hit ? <mark key={i}>{p.t}</mark> : <span key={i}>{p.t}</span>))}
    </>
  );
}
