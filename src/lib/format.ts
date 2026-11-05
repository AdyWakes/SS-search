export function formatBytes(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)} GB`;
  if (n >= 1e6) return `${Math.round(n / 1e6)} MB`;
  if (n >= 1e3) return `${Math.round(n / 1e3)} KB`;
  return `${n} B`;
}

export function formatCount(n: number): string {
  return n.toLocaleString();
}

const DAY = 86_400_000;

export function formatDate(ms: number, now = Date.now()): string {
  const d = new Date(ms);
  const today = new Date(now);
  today.setHours(0, 0, 0, 0);
  const t = today.getTime();
  const time = d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  if (ms >= t) return `Today, ${time}`;
  if (ms >= t - DAY) return `Yesterday, ${time}`;
  if (ms >= t - 6 * DAY) return d.toLocaleDateString(undefined, { weekday: "long" });
  if (d.getFullYear() === today.getFullYear()) return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  return d.toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}

export function shortenPath(p: string, max = 48): string {
  if (p.length <= max) return p;
  const sep = p.includes("\\") ? "\\" : "/";
  const parts = p.split(sep);
  let out = parts[parts.length - 1];
  for (let i = parts.length - 2; i >= 0; i--) {
    const next = parts[i] + sep + out;
    if (next.length + 2 > max) return "…" + sep + out;
    out = next;
  }
  return out;
}

export const isMac = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform || navigator.userAgent);
export const mod = isMac ? "⌘" : "Ctrl";

/** Splits `text` into [plain, match, plain, ...] for highlighting query words. */
export function highlight(text: string, query: string): { t: string; hit: boolean }[] {
  const words = query
    .toLowerCase()
    .split(/[^\p{L}\p{N}]+/u)
    .filter((w) => w.length >= 2);
  if (!words.length) return [{ t: text, hit: false }];
  const esc = words.map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  const re = new RegExp(`(${esc.join("|")})`, "giu");
  const out: { t: string; hit: boolean }[] = [];
  let last = 0;
  for (const m of text.matchAll(re)) {
    const i = m.index ?? 0;
    if (i > last) out.push({ t: text.slice(last, i), hit: false });
    out.push({ t: m[0], hit: true });
    last = i + m[0].length;
  }
  if (last < text.length) out.push({ t: text.slice(last), hit: false });
  return out;
}

/** Trims a long snippet so the first matching word is visible in one line. */
export function focusSnippet(text: string, query: string, lead = 24): string {
  const parts = highlight(text, query);
  let offset = 0;
  for (const p of parts) {
    if (p.hit) break;
    offset += p.t.length;
  }
  if (offset >= text.length || offset <= lead) return text;
  const cut = text.lastIndexOf(" ", offset - lead);
  return "…" + text.slice(cut >= 0 ? cut + 1 : offset - lead);
}
