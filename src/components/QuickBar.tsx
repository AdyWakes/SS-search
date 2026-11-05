import { useEffect, useRef, useState } from "react";
import { useApi } from "../lib/ApiContext";
import { useSearch } from "../lib/useSearch";
import { handleResultKeys, useActions } from "../lib/useActions";
import { ResultGrid, gridColumns } from "./ResultGrid";
import { SearchIcon } from "./icons";
import { mod } from "../lib/format";

const BAR_H = 76;
const COLS = 4;
// Header 74 + padding 32 + footer 36; a compact tile row (171px wide) is ~107 + 22 label + 12 gap.
const heightFor = (n: number) => (n === 0 ? 180 : Math.min(500, 74 + 32 + 36 + Math.ceil(n / COLS) * 141));

/** The Spotlight-style window opened by the global hotkey. */
export function QuickBar() {
  const api = useApi();
  const act = useActions();
  const [text, setText] = useState("");
  const [sel, setSel] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const box = useRef<HTMLDivElement>(null);
  const { results } = useSearch(api, text, { screenshots_only: false }, 12);
  const hits = text.trim() ? (results?.hits ?? []) : [];

  useEffect(() => {
    const u = api.onBarOpened(() => {
      setText("");
      setSel(0);
      setTimeout(() => input.current?.focus(), 10);
    });
    input.current?.focus();
    return () => void u.then((f) => f());
  }, [api]);
  useEffect(() => setSel(0), [text]);
  useEffect(() => {
    api.setWindowHeight(text.trim() ? heightFor(hits.length) : BAR_H);
  }, [api, hits.length, text]);

  const onKey = (e: React.KeyboardEvent) => {
    const cols = gridColumns(box.current);
    if (e.key === "Escape") {
      e.preventDefault();
      if (text) setText("");
      else api.hideBar();
      return;
    }
    if (e.key === "Tab") {
      e.preventDefault();
      api.showMain(text);
      return;
    }
    const move = { ArrowRight: 1, ArrowLeft: -1, ArrowDown: cols, ArrowUp: -cols }[e.key];
    if (move && hits.length) {
      e.preventDefault();
      setSel((s) => Math.min(hits.length - 1, Math.max(0, s + move)));
      return;
    }
    const id = hits[sel]?.id ?? null;
    if (handleResultKeys(e, id, act)) {
      e.preventDefault();
      if (e.key === "Enter") api.hideBar();
    }
  };

  return (
    <div className="flex h-full flex-col overflow-hidden border border-line bg-bg" onKeyDown={onKey}>
      <div className="flex h-[74px] shrink-0 items-center gap-4 px-6" data-tauri-drag-region>
        <SearchIcon size={24} className="shrink-0 text-ink-2" />
        <input
          ref={input}
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder="Search your screenshots"
          spellCheck={false}
          autoFocus
          aria-label="Search"
          className="h-full min-w-0 flex-1 bg-transparent font-display text-[24px] text-ink outline-none placeholder:text-ink-3"
        />
        <kbd>Esc</kbd>
      </div>
      {text.trim() && (
        <>
          <div ref={box} className="scroll min-h-0 flex-1 border-t border-line px-5 py-4">
            {hits.length ? (
              <ResultGrid
                compact
                hits={hits}
                query={text}
                selected={sel}
                onSelect={setSel}
                onOpen={(i) => {
                  act.open(hits[i].id);
                  api.hideBar();
                }}
                onPreview={() => api.showMain(text)}
              />
            ) : (
              <p className="px-1 py-6 text-[15px] text-ink-2">{results ? "Nothing matches yet." : ""}</p>
            )}
          </div>
          <div className="flex h-9 shrink-0 items-center gap-4 border-t border-line px-5 text-[12.5px] text-ink-2">
            <span>
              <kbd>Enter</kbd> open
            </span>
            <span>
              <kbd>{mod}+C</kbd> copy image
            </span>
            <span>
              <kbd>{mod}+Shift+C</kbd> copy text
            </span>
            <span className="ml-auto">
              <kbd>Tab</kbd> all results
            </span>
          </div>
        </>
      )}
    </div>
  );
}
