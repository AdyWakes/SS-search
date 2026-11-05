import { useEffect, useState } from "react";
import type { FileDetails, Hit } from "../lib/types";
import { useApi } from "../lib/ApiContext";
import { useActions } from "../lib/useActions";
import { formatBytes, formatDate, mod } from "../lib/format";
import { CloseIcon } from "./icons";

export function Preview({ hit, onClose }: { hit: Hit; onClose: () => void }) {
  const api = useApi();
  const act = useActions();
  const [details, setDetails] = useState<FileDetails | null>(null);
  const [full, setFull] = useState(false);
  useEffect(() => {
    setDetails(null);
    setFull(false);
    api.details(hit.id).then(setDetails).catch(() => {});
  }, [api, hit.id]);
  const f = details?.file ?? hit;
  return (
    <div className="fixed inset-0 z-40 flex bg-scrim backdrop-blur-[2px]" onMouseDown={onClose} role="dialog" aria-label={`Preview of ${hit.name}`}>
      <div className="m-6 flex min-h-0 flex-1 overflow-hidden rounded-xl border border-line bg-bg shadow-2xl" onMouseDown={(e) => e.stopPropagation()}>
        <div className="flex min-w-0 flex-1 items-center justify-center bg-sunken p-6">
          <img
            src={full ? api.fullUrl(hit.id) : api.thumbUrl(hit.id)}
            onLoad={() => setFull(true)}
            alt={hit.name}
            draggable={false}
            className="max-h-full max-w-full rounded-md object-contain shadow-lg"
          />
        </div>
        <aside className="flex w-[340px] shrink-0 flex-col border-l border-line">
          <div className="flex items-start justify-between gap-3 p-5 pb-3">
            <h2 className="selectable font-display text-[18px] font-semibold leading-6 break-all">{f.name}</h2>
            <button type="button" onClick={onClose} aria-label="Close preview" className="-mr-1 rounded-md p-1 text-ink-2 hover:bg-raised hover:text-ink">
              <CloseIcon />
            </button>
          </div>
          <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 px-5 text-[13.5px]">
            <dt className="text-ink-2">Folder</dt>
            <dd className="selectable truncate" title={f.dir}>{f.dir}</dd>
            <dt className="text-ink-2">Modified</dt>
            <dd>{formatDate(f.mtime_ms)}</dd>
            {f.width && (
              <>
                <dt className="text-ink-2">Size</dt>
                <dd>
                  {f.width} × {f.height} · {formatBytes(f.size)}
                </dd>
              </>
            )}
          </dl>
          <div className="mt-5 flex flex-wrap gap-2 px-5">
            <ActionButton primary onClick={() => act.open(hit.id)} label="Open" k="Enter" />
            <ActionButton onClick={() => act.reveal(hit.id)} label="Show in folder" k={`${mod}+Enter`} />
            <ActionButton onClick={() => act.copyImage(hit.id)} label="Copy image" k={`${mod}+C`} />
            <ActionButton onClick={() => act.copyText(hit.id)} label="Copy text" k={`${mod}+Shift+C`} />
          </div>
          <h3 className="mt-6 px-5 text-[13.5px] font-semibold">Text in this image</h3>
          <div className="scroll selectable mt-2 min-h-0 flex-1 whitespace-pre-wrap px-5 pb-5 text-[13.5px] leading-[1.55] text-ink-2">
            {details == null ? "…" : details.text.trim() ? details.text : "No text found."}
          </div>
        </aside>
      </div>
    </div>
  );
}

function ActionButton({ label, k, onClick, primary }: { label: string; k: string; onClick: () => void; primary?: boolean }) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={k}
      className={`h-8 rounded-md px-3 text-[13.5px] transition-colors ${
        primary ? "bg-accent text-accent-ink hover:brightness-110" : "border border-line hover:bg-raised"
      }`}
    >
      {label}
    </button>
  );
}
