import { useEffect, useState, type ReactNode } from "react";
import type { AppInfo, Folder, IndexingSpeed, ModelsView, Settings as S, SettingsView, Status } from "../lib/types";
import { useApi } from "../lib/ApiContext";
import { useToast } from "./Toast";
import { formatBytes, formatCount } from "../lib/format";
import { CheckIcon, CloseIcon, FolderIcon } from "./icons";

const KIND: Record<Folder["kind"], string> = {
  screenshots: "Screenshots",
  pictures: "Pictures",
  downloads: "Downloads",
  desktop: "Desktop",
  custom: "Added by you",
};

export function SettingsPage({ status, onModels }: { status: Status | null; onModels: () => void }) {
  const api = useApi();
  const toast = useToast();
  const [view, setView] = useState<SettingsView | null>(null);
  const [models, setModels] = useState<ModelsView | null>(null);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [confirmWipe, setConfirmWipe] = useState(false);

  useEffect(() => {
    api.getSettings().then(setView);
    api.models().then(setModels);
    api.appInfo().then(setInfo);
  }, [api]);

  if (!view) return null;
  const s = view.settings;
  const save = async (next: S) => {
    setView({ ...view, settings: next });
    try {
      await api.saveSettings(next);
    } catch (e) {
      toast(String(e), true);
      api.getSettings().then(setView);
    }
  };
  const addFolder = async () => {
    const p = await api.pickFolder();
    if (!p || s.folders.some((f) => f.path === p)) return;
    save({ ...s, folders: [...s.folders, { path: p, kind: "custom", enabled: true }] });
  };

  return (
    <div className="scroll h-full">
      <div className="mx-auto max-w-[720px] px-8 pb-24 pt-8">
        <Section title="Folders" desc="shotfind indexes images in these folders and watches them for new screenshots. Your files are only ever read, never changed, moved or deleted.">
          <ul className="divide-y divide-line border-y border-line">
            {s.folders.map((f, i) => (
              <li key={f.path} className="flex items-center gap-3 py-3">
                <FolderIcon className="shrink-0 text-ink-2" />
                <div className="min-w-0 flex-1">
                  <div className={`truncate text-[14px] ${f.enabled ? "" : "text-ink-3 line-through"}`} title={f.path}>
                    {f.path}
                  </div>
                  <div className="text-[13px] text-ink-2">{KIND[f.kind]}</div>
                </div>
                <Toggle
                  checked={f.enabled}
                  label={`Index ${f.path}`}
                  onChange={(v) => save({ ...s, folders: s.folders.map((x, j) => (j === i ? { ...x, enabled: v } : x)) })}
                />
                <button
                  type="button"
                  aria-label={`Remove ${f.path}`}
                  onClick={() => save({ ...s, folders: s.folders.filter((_, j) => j !== i) })}
                  className="rounded-md p-1.5 text-ink-2 hover:bg-raised hover:text-ink"
                >
                  <CloseIcon size={16} />
                </button>
              </li>
            ))}
          </ul>
          <div className="mt-3 flex gap-2">
            <Button onClick={addFolder}>Add folder…</Button>
            <Button onClick={() => save({ ...s, folders: view.defaults })}>Reset to defaults</Button>
          </div>
        </Section>

        <Section title="Search models" desc="Run entirely on this computer. Downloaded once, verified by SHA-256.">
          {models?.runtime_error && <p className="mb-3 text-[14px] text-accent">The ML runtime couldn’t start: {models.runtime_error}</p>}
          <ul className="divide-y divide-line border-y border-line">
            {models?.models.map((m) => (
              <li key={m.id} className="flex items-center gap-4 py-3">
                <div className="min-w-0 flex-1">
                  <div className="text-[14px] font-semibold">{m.title}</div>
                  <div className="text-[13px] text-ink-2">
                    {formatBytes(m.disk_bytes)} on disk · {m.license}
                  </div>
                </div>
                {m.installed ? (
                  <span className="inline-flex items-center gap-1 text-[14px]">
                    <CheckIcon size={15} /> Installed
                  </span>
                ) : m.pinned ? (
                  <Button onClick={onModels}>Download {formatBytes(m.download_bytes)}</Button>
                ) : (
                  <span className="text-[13px] text-ink-2">Not available in this build</span>
                )}
              </li>
            ))}
          </ul>
          <div className="mt-4">
            <Row label="Meaning search" hint="Find images by what they show, not only by their text.">
              <Toggle checked={s.semantic_enabled} label="Meaning search" onChange={(v) => save({ ...s, semantic_enabled: v })} />
            </Row>
          </div>
        </Section>

        <Section title="Indexing" desc="Runs in the background at low priority and picks up where it left off after a restart.">
          <Row label="Speed" hint="How much of your processor indexing may use.">
            <Segmented
              value={s.indexing_speed}
              options={[
                ["gentle", "Gentle"],
                ["balanced", "Balanced"],
                ["fast", "Fast"],
              ]}
              onChange={(v) => save({ ...s, indexing_speed: v as IndexingSpeed })}
            />
          </Row>
          <Row label="Skip images smaller than" hint="Ignores icons and tiny thumbnails.">
            <NumberField value={s.min_dimension} suffix="px" onChange={(v) => save({ ...s, min_dimension: v })} />
          </Row>
          <Row label="Skip files larger than" hint="Very large files are usually photos or scans.">
            <NumberField value={Math.round(s.max_file_bytes / 1048576)} suffix="MB" onChange={(v) => save({ ...s, max_file_bytes: v * 1048576 })} />
          </Row>
          {status && (
            <p className="mt-3 text-[13.5px] text-ink-2">
              {formatCount(status.counts.done)} indexed · {formatCount(status.counts.pending)} waiting · {formatCount(status.counts.skipped)} skipped
              {status.counts.failed ? ` · ${formatCount(status.counts.failed)} couldn’t be read` : ""}
            </p>
          )}
          <div className="mt-3 flex flex-wrap gap-2">
            <Button onClick={() => api.rescan().then(() => toast("Rescanning folders"))}>Rescan now</Button>
            {!!status?.counts.failed && <Button onClick={() => api.retryFailed().then((n) => toast(`Retrying ${n} files`))}>Retry failed</Button>}
            {status?.phase === "paused" ? (
              <Button onClick={() => api.resume()}>Resume indexing</Button>
            ) : (
              <Button onClick={() => api.pause()}>Pause indexing</Button>
            )}
          </div>
        </Section>

        <Section title="Shortcut" desc="Opens the quick search bar from anywhere.">
          <HotkeyField value={s.hotkey} onChange={(v) => save({ ...s, hotkey: v })} />
        </Section>

        <Section title="Privacy" desc="shotfind has no account, no telemetry and no network access after the model download. The index (text, thumbnails, vectors) lives only on this computer.">
          <div className="text-[13.5px] text-ink-2">
            Index location: <span className="selectable text-ink">{view.data_dir}</span>
          </div>
          <div className="mt-3">
            {confirmWipe ? (
              <div className="flex flex-wrap items-center gap-3">
                <span className="text-[14px]">Delete the whole index? Your images are not affected.</span>
                <Button
                  danger
                  onClick={async () => {
                    setConfirmWipe(false);
                    await api.wipeIndex();
                    toast("Index deleted. Re-indexing…");
                  }}
                >
                  Delete index
                </Button>
                <Button onClick={() => setConfirmWipe(false)}>Cancel</Button>
              </div>
            ) : (
              <Button onClick={() => setConfirmWipe(true)}>Delete index…</Button>
            )}
          </div>
        </Section>

        <Section title="About">
          <p className="text-[13.5px] leading-[1.6] text-ink-2">
            shotfind {info?.version} · open source (MIT)
            <br />
            OCR: PaddleOCR PP-OCRv4 (Apache-2.0) · Meaning search: OpenAI CLIP ViT-B/32 (MIT) · ONNX Runtime (MIT)
          </p>
        </Section>
      </div>
    </div>
  );
}

function Section({ title, desc, children }: { title: string; desc?: string; children: ReactNode }) {
  return (
    <section className="mt-10 first:mt-0">
      <h2 className="font-display text-[20px] font-semibold">{title}</h2>
      {desc && <p className="mt-1 max-w-[600px] text-[14px] leading-[1.55] text-ink-2">{desc}</p>}
      <div className="mt-4">{children}</div>
    </section>
  );
}

function Row({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-6 py-2.5">
      <div>
        <div className="text-[14px]">{label}</div>
        {hint && <div className="text-[13px] text-ink-2">{hint}</div>}
      </div>
      {children}
    </div>
  );
}

function Button({ children, onClick, danger }: { children: ReactNode; onClick: () => void; danger?: boolean }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`h-8 rounded-md px-3 text-[13.5px] transition-colors ${
        danger ? "bg-accent font-semibold text-accent-ink hover:brightness-110" : "border border-line hover:bg-raised"
      }`}
    >
      {children}
    </button>
  );
}

function Toggle({ checked, onChange, label }: { checked: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={() => onChange(!checked)}
      className={`relative h-6 w-10 shrink-0 rounded-full transition-colors ${checked ? "bg-accent" : "bg-line"}`}
    >
      <span className={`absolute top-0.5 h-5 w-5 rounded-full bg-white shadow transition-[left] ${checked ? "left-[18px]" : "left-0.5"}`} />
    </button>
  );
}

function Segmented({ value, options, onChange }: { value: string; options: [string, string][]; onChange: (v: string) => void }) {
  return (
    <div role="radiogroup" className="flex rounded-lg border border-line p-0.5">
      {options.map(([v, l]) => (
        <button
          key={v}
          type="button"
          role="radio"
          aria-checked={v === value}
          onClick={() => onChange(v)}
          className={`h-7 rounded-md px-3 text-[13.5px] ${v === value ? "bg-raised text-ink shadow-sm" : "text-ink-2 hover:text-ink"}`}
        >
          {l}
        </button>
      ))}
    </div>
  );
}

function NumberField({ value, suffix, onChange }: { value: number; suffix: string; onChange: (v: number) => void }) {
  const [v, setV] = useState(String(value));
  useEffect(() => setV(String(value)), [value]);
  return (
    <label className="flex items-center gap-2 text-[14px]">
      <input
        type="number"
        min={1}
        value={v}
        onChange={(e) => setV(e.target.value)}
        onBlur={() => {
          const n = Math.max(1, Math.round(Number(v) || value));
          if (n !== value) onChange(n);
          setV(String(n));
        }}
        className="h-8 w-20 rounded-md border border-line bg-raised px-2 text-right tabular-nums outline-none focus:border-accent"
      />
      <span className="text-ink-2">{suffix}</span>
    </label>
  );
}

function HotkeyField({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const [recording, setRecording] = useState(false);
  return (
    <div className="flex items-center gap-3">
      <button
        type="button"
        onClick={() => setRecording(true)}
        onBlur={() => setRecording(false)}
        onKeyDown={(e) => {
          if (!recording) return;
          e.preventDefault();
          if (e.key === "Escape") return setRecording(false);
          if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return;
          const parts = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(Boolean) as string[];
          if (!parts.length) return;
          const key = e.code.startsWith("Key") ? e.code.slice(3) : e.code.startsWith("Digit") ? e.code.slice(5) : e.code;
          onChange([...parts, key].join("+"));
          setRecording(false);
        }}
        className={`h-9 min-w-48 rounded-md border px-3 text-left text-[14px] ${recording ? "border-accent text-ink-2" : "border-line"}`}
      >
        {recording ? "Press the new shortcut…" : value.split("+").map((k) => <kbd key={k} className="mr-1">{k}</kbd>)}
      </button>
      <button type="button" onClick={() => onChange("Alt+Shift+S")} className="text-[13.5px] text-ink-2 hover:text-ink">
        Reset
      </button>
    </div>
  );
}
