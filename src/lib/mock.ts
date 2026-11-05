// In-browser stand-in for the Rust backend: used by `npm run dev` outside Tauri
// and by the Playwright UI screenshots. Images are generated SVGs.
import type { Api } from "./api";
import type {
  DownloadProgress,
  Hit,
  ModelId,
  ModelsView,
  SearchResults,
  Settings,
  SettingsView,
  Status,
} from "./types";

const W = 640;
const H = 400;
const svg = (body: string, w = W, h = H) =>
  `data:image/svg+xml;utf8,${encodeURIComponent(
    `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}" font-family="Segoe UI, Helvetica, Arial, sans-serif">${body}</svg>`,
  )}`;
const mono = `font-family="Consolas, Menlo, monospace"`;
const lines = (x: number, y: number, n: number, color: string, widths: number[], gap = 18, h = 7) =>
  Array.from({ length: n }, (_, i) => `<rect x="${x}" y="${y + i * gap}" width="${widths[i % widths.length]}" height="${h}" rx="3" fill="${color}"/>`).join("");

const art = {
  chat: (name: string, msg: string, amount: string) =>
    svg(`<rect width="640" height="400" fill="#eef1f4"/><rect width="640" height="54" fill="#ffffff"/>
      <circle cx="34" cy="27" r="16" fill="#c9d3dd"/><text x="60" y="33" font-size="17" font-weight="600" fill="#1d2329">${name}</text>
      <rect x="24" y="78" width="300" height="44" rx="14" fill="#ffffff"/><text x="40" y="105" font-size="15" fill="#20262c">Did you get the money for dinner?</text>
      <rect x="250" y="140" width="366" height="76" rx="14" fill="#d8f0d2"/><text x="266" y="168" font-size="15" fill="#1c2a19">${msg}</text>
      <text x="266" y="196" font-size="22" font-weight="700" fill="#1c2a19">${amount}</text>
      <rect x="24" y="236" width="230" height="44" rx="14" fill="#ffffff"/><text x="40" y="263" font-size="15" fill="#20262c">Thanks! Got it 🙌</text>
      <rect x="0" y="346" width="640" height="54" fill="#ffffff"/><rect x="18" y="358" width="560" height="30" rx="15" fill="#eef1f4"/>`),
  terminal: (l: string[]) =>
    svg(`<rect width="640" height="400" fill="#16181b"/><rect width="640" height="30" fill="#24272b"/>
      <circle cx="18" cy="15" r="6" fill="#ff5f57"/><circle cx="38" cy="15" r="6" fill="#febc2e"/><circle cx="58" cy="15" r="6" fill="#28c840"/>
      ${l.map((t, i) => `<text x="18" y="${64 + i * 24}" font-size="14" ${mono} fill="${t.startsWith("error") ? "#ff7b72" : t.startsWith("$") ? "#7ee787" : "#c9d1d9"}">${t}</text>`).join("")}`),
  receipt: (shop: string, total: string) =>
    svg(`<rect width="640" height="400" fill="#d9d4cc"/><rect x="190" y="18" width="260" height="364" fill="#fffdf8"/>
      <text x="320" y="58" font-size="18" font-weight="700" text-anchor="middle" fill="#222">${shop}</text>
      ${lines(214, 84, 8, "#c8c3ba", [150, 120, 170, 100, 140, 160, 110, 130], 22, 6)}
      <rect x="214" y="270" width="212" height="1.5" fill="#999"/>
      <text x="214" y="300" font-size="16" font-weight="700" fill="#222">TOTAL</text><text x="426" y="300" font-size="16" font-weight="700" text-anchor="end" fill="#222">${total}</text>
      <text x="320" y="350" font-size="11" text-anchor="middle" fill="#777">Thank you for shopping</text>`),
  code: () =>
    svg(`<rect width="640" height="400" fill="#1e1f22"/><rect width="150" height="400" fill="#18191b"/>
      ${lines(18, 26, 12, "#3a3d42", [90, 70, 100, 60, 80, 96, 70, 84, 66, 92, 74, 88], 28, 8)}
      ${Array.from({ length: 13 }, (_, i) => {
        const c = ["#c678dd", "#61afef", "#98c379", "#e5c07b", "#abb2bf"];
        return `<rect x="${176 + (i % 3) * 22}" y="${28 + i * 27}" width="${60 + ((i * 47) % 200)}" height="9" rx="3" fill="${c[i % 5]}" opacity=".85"/><rect x="${250 + ((i * 47) % 200)}" y="${28 + i * 27}" width="${40 + ((i * 31) % 90)}" height="9" rx="3" fill="#abb2bf" opacity=".5"/>`;
      }).join("")}`),
  chart: (title: string) =>
    svg(`<rect width="640" height="400" fill="#ffffff"/><text x="28" y="44" font-size="20" font-weight="650" fill="#1a1a1a">${title}</text>
      ${[160, 220, 130, 260, 200, 290, 240].map((h, i) => `<rect x="${48 + i * 82}" y="${360 - h}" width="52" height="${h}" rx="4" fill="${i === 5 ? "#ff6b2c" : "#cfd4da"}"/>`).join("")}
      <rect x="28" y="360" width="590" height="2" fill="#1a1a1a"/>`),
  whiteboard: (t1: string, t2: string) =>
    svg(`<rect width="640" height="400" fill="#8c8174"/><rect x="30" y="22" width="580" height="356" rx="6" fill="#f4f3ef"/>
      <text x="70" y="96" font-size="38" fill="#2552c4" font-family="Segoe Print, Comic Sans MS, cursive" transform="rotate(-2 70 96)">${t1}</text>
      <text x="80" y="170" font-size="26" fill="#c62f2f" font-family="Segoe Print, Comic Sans MS, cursive">${t2}</text>
      <path d="M80 230 q60 -30 120 0 t120 0" stroke="#2a7a3a" stroke-width="5" fill="none"/>
      <rect x="380" y="200" width="170" height="120" rx="6" stroke="#2552c4" stroke-width="4" fill="none"/>
      <text x="400" y="268" font-size="24" fill="#2552c4" font-family="Segoe Print, Comic Sans MS, cursive">ship it!</text>`),
  cat: (caption: string) =>
    svg(`<rect width="640" height="400" fill="#f1e3cf"/>
      <text x="320" y="56" font-size="34" font-weight="900" text-anchor="middle" fill="#fff" stroke="#000" stroke-width="2" font-family="Impact, Arial Black, sans-serif">${caption}</text>
      <ellipse cx="320" cy="300" rx="130" ry="90" fill="#e08a3c"/><circle cx="320" cy="200" r="78" fill="#e9973f"/>
      <path d="M258 150 l-14 -62 l52 36z M382 150 l14 -62 l-52 36z" fill="#e9973f"/>
      <circle cx="292" cy="196" r="11" fill="#2b2b2b"/><circle cx="348" cy="196" r="11" fill="#2b2b2b"/>
      <path d="M312 222 h16 l-8 9z" fill="#c85a6a"/><path d="M262 226 h-46 M262 236 h-44 M378 226 h46 M378 236 h44" stroke="#5a3a1a" stroke-width="2"/>`),
  notFound: () =>
    svg(`<rect width="640" height="400" fill="#fbfbfb"/><rect width="640" height="40" fill="#f0f0f0"/><rect x="90" y="10" width="460" height="20" rx="10" fill="#fff"/>
      <text x="320" y="196" font-size="96" font-weight="800" text-anchor="middle" fill="#222">404</text>
      <text x="320" y="240" font-size="20" text-anchor="middle" fill="#555">Page not found</text>
      <rect x="250" y="270" width="140" height="38" rx="8" fill="#222"/><text x="320" y="295" font-size="14" text-anchor="middle" fill="#fff">Back to home</text>`),
  email: (subject: string) =>
    svg(`<rect width="640" height="400" fill="#ffffff"/><rect width="160" height="400" fill="#f3f4f6"/>
      ${lines(20, 30, 7, "#c9ccd1", [100, 80, 110, 70, 90, 60, 100], 34, 10)}
      <text x="184" y="44" font-size="19" font-weight="650" fill="#111">${subject}</text>
      ${lines(184, 74, 11, "#d7dade", [400, 380, 420, 300, 410, 390, 260, 400, 350, 380, 200], 26, 9)}`),
  map: () =>
    svg(`<rect width="640" height="400" fill="#e9efe4"/><path d="M0 120 C200 80 380 220 640 160" stroke="#fff" stroke-width="18" fill="none"/>
      <path d="M220 0 C240 140 160 260 260 400" stroke="#fff" stroke-width="14" fill="none"/><path d="M0 300 L640 270" stroke="#f6d77a" stroke-width="12"/>
      <rect x="420" y="40" width="140" height="90" fill="#cfe2c2"/><circle cx="352" cy="190" r="16" fill="#ff6b2c"/><circle cx="352" cy="190" r="6" fill="#fff"/>
      <rect x="20" y="20" width="250" height="44" rx="10" fill="#fff"/><text x="36" y="48" font-size="15" fill="#333">Riverside Cafe, 12 Main St</text>`),
  boarding: (route: string) =>
    svg(`<rect width="640" height="400" fill="#203a5c"/><rect x="40" y="40" width="560" height="320" rx="18" fill="#ffffff"/>
      <text x="80" y="110" font-size="44" font-weight="800" fill="#1b2b40">${route}</text><text x="80" y="150" font-size="16" fill="#5b6b80">Boarding pass · Seat 14C · Gate B22</text>
      ${Array.from({ length: 36 }, (_, i) => `<rect x="${80 + i * 13}" y="230" width="${i % 3 ? 6 : 9}" height="90" fill="#1b2b40"/>`).join("")}`),
  phone: (title: string, body: string) =>
    svg(`<rect width="360" height="740" fill="#101114"/><rect x="0" y="0" width="360" height="40" fill="#101114"/>
      <text x="24" y="28" font-size="14" fill="#fff">9:41</text>
      <rect x="16" y="70" width="328" height="96" rx="20" fill="#2a2c31"/><text x="34" y="106" font-size="15" font-weight="600" fill="#fff">${title}</text>
      <text x="34" y="134" font-size="14" fill="#c5c7cc">${body}</text>
      ${Array.from({ length: 8 }, (_, i) => `<rect x="${30 + (i % 4) * 80}" y="${520 + Math.floor(i / 4) * 92}" width="56" height="56" rx="14" fill="${["#3c3f46", "#4a4e57", "#353840", "#41454d"][i % 4]}"/>`).join("")}`, 360, 740),
  sheet: () =>
    svg(`<rect width="640" height="400" fill="#ffffff"/><rect width="640" height="36" fill="#f3f3f3"/>
      ${Array.from({ length: 14 }, (_, r) => `<rect x="0" y="${36 + r * 26}" width="640" height="1" fill="#e2e2e2"/>`).join("")}
      ${Array.from({ length: 6 }, (_, c) => `<rect x="${c * 108}" y="36" width="1" height="364" fill="#e2e2e2"/>`).join("")}
      ${Array.from({ length: 13 }, (_, r) => Array.from({ length: 5 }, (_, c) => `<rect x="${c * 108 + 10}" y="${46 + r * 26}" width="${30 + ((r * 13 + c * 29) % 60)}" height="7" rx="2" fill="${c === 4 ? "#2e7d32" : "#9aa0a6"}"/>`).join("")).join("")}`),
};

interface MockItem {
  name: string;
  dir: string;
  text: string;
  tags: string[];
  img: string;
  daysAgo: number;
  w: number;
  h: number;
}

const SS = "C:\\Users\\Asha\\Pictures\\Screenshots";
const DL = "C:\\Users\\Asha\\Downloads";
const PX = "C:\\Users\\Asha\\Pictures";

const ITEMS: MockItem[] = [
  { name: "Screenshot 2026-11-04 213012.png", dir: SS, text: "Rahul Sharma\nDid you get the money for dinner?\nUPI payment received from Rahul\n₹1,250.00\nThanks! Got it", tags: ["receipt", "payment", "chat"], img: art.chat("Rahul Sharma", "UPI payment received from Rahul", "₹1,250.00"), daysAgo: 0.2, w: 1920, h: 1080 },
  { name: "Screenshot 2026-11-04 101544.png", dir: SS, text: "$ cargo build --release\nCompiling shotfind v0.1.0\nerror[E0425]: cannot find value `config`\n--> src/main.rs:42:13", tags: ["code", "terminal", "error"], img: art.terminal(["$ cargo build --release", "   Compiling shotfind v0.1.0", "error[E0425]: cannot find value `config`", "  --> src/main.rs:42:13", "warning: unused variable: `x`", "$ git status"]), daysAgo: 0.6, w: 1920, h: 1080 },
  { name: "Screenshot 2026-11-03 184420.png", dir: SS, text: "404\nPage not found\nBack to home", tags: ["error", "website"], img: art.notFound(), daysAgo: 1.3, w: 2560, h: 1600 },
  { name: "IMG_20261102_whiteboard.jpg", dir: PX, text: "Q3 roadmap\nsearch first, then sync\nship it!", tags: ["whiteboard", "notes", "meeting"], img: art.whiteboard("Q3 roadmap", "search first, then sync"), daysAgo: 2.1, w: 4032, h: 3024 },
  { name: "when-the-build-passes.jpg", dir: DL, text: "WHEN THE BUILD PASSES", tags: ["cat", "meme", "funny"], img: art.cat("WHEN THE BUILD PASSES"), daysAgo: 2.5, w: 1200, h: 750 },
  { name: "receipt-greenleaf-market.png", dir: DL, text: "GREENLEAF MARKET\nTOTAL ₹2,318.40\nThank you for shopping", tags: ["receipt", "shopping", "groceries"], img: art.receipt("GREENLEAF MARKET", "₹2,318.40"), daysAgo: 3.2, w: 1080, h: 1920 },
  { name: "Screenshot 2026-11-01 090211.png", dir: SS, text: "Weekly active users\nOctober", tags: ["chart", "dashboard", "graph"], img: art.chart("Weekly active users"), daysAgo: 4, w: 1920, h: 1080 },
  { name: "Screenshot 2026-10-31 162300.png", dir: SS, text: "Invoice #2026-117 from Northwind Design\nPayment due Nov 15", tags: ["email", "invoice"], img: art.email("Invoice #2026-117 from Northwind Design"), daysAgo: 5, w: 1920, h: 1080 },
  { name: "Screenshot 2026-10-30 080055.png", dir: SS, text: "Riverside Cafe, 12 Main St\nDirections", tags: ["map", "location"], img: art.map(), daysAgo: 6, w: 1920, h: 1080 },
  { name: "boarding-pass-BLR-DEL.png", dir: DL, text: "BLR → DEL\nBoarding pass Seat 14C Gate B22", tags: ["ticket", "flight", "travel"], img: art.boarding("BLR → DEL"), daysAgo: 8, w: 1600, h: 1000 },
  { name: "Screenshot_20261027-221530.png", dir: SS, text: "Bank alert\n₹4,999 debited for subscription renewal", tags: ["phone", "notification", "payment"], img: art.phone("Bank alert", "₹4,999 debited · renewal"), daysAgo: 9, w: 1080, h: 2220 },
  { name: "Screenshot 2026-10-25 140902.png", dir: SS, text: "fn main() {\nlet config = load_config()?;\n}", tags: ["code", "editor"], img: art.code(), daysAgo: 11, w: 2560, h: 1440 },
  { name: "budget-2026.png", dir: PX, text: "Budget 2026\nRent Groceries Travel Savings", tags: ["spreadsheet", "table", "budget"], img: art.sheet(), daysAgo: 13, w: 1920, h: 1080 },
  { name: "Screenshot 2026-10-21 193311.png", dir: SS, text: "Priya Patel\nUPI payment of ₹600 for movie tickets", tags: ["chat", "payment"], img: art.chat("Priya Patel", "UPI payment of ₹600 for tickets", "₹600.00"), daysAgo: 15, w: 1920, h: 1080 },
  { name: "Screenshot 2026-10-18 110040.png", dir: SS, text: "$ npm run build\nerror TS2304: Cannot find name 'useStat'", tags: ["code", "terminal", "error"], img: art.terminal(["$ npm run build", "error TS2304: Cannot find name 'useStat'", "  src/App.tsx:12:5", "Found 1 error."]), daysAgo: 18, w: 1920, h: 1080 },
  { name: "retro-board.jpg", dir: PX, text: "Sprint retro\nwhat went well", tags: ["whiteboard", "notes"], img: art.whiteboard("Sprint retro", "what went well"), daysAgo: 22, w: 4032, h: 3024 },
  { name: "Screenshot 2026-10-10 151515.png", dir: SS, text: "Revenue by month", tags: ["chart", "graph"], img: art.chart("Revenue by month"), daysAgo: 26, w: 1920, h: 1080 },
  { name: "cafe-bill.jpg", dir: DL, text: "BLUE DOOR COFFEE\nTOTAL ₹540.00", tags: ["receipt", "food"], img: art.receipt("BLUE DOOR COFFEE", "₹540.00"), daysAgo: 31, w: 1080, h: 1920 },
];

const NOW = Date.now();
const DAY = 86_400_000;

function hitOf(i: number, kind: Hit["kind"], score: number, snippet: string | null): Hit {
  const it = ITEMS[i];
  return {
    id: i + 1,
    path: `${it.dir}\\${it.name}`,
    dir: it.dir,
    name: it.name,
    size: 180_000 + i * 7919,
    mtime_ms: NOW - it.daysAgo * DAY,
    width: it.w,
    height: it.h,
    is_screenshot: it.dir === SS,
    state: 1,
    score,
    text_score: kind === "meaning" ? 0 : score,
    semantic_score: kind === "text" ? 0 : score,
    kind,
    snippet,
  };
}

const words = (s: string) => s.toLowerCase().split(/[^\p{L}\p{N}]+/u).filter(Boolean);
function near(a: string, b: string): boolean {
  if (a === b || (a.length >= 2 && b.startsWith(a))) return true;
  if (a.length < 4 || Math.abs(a.length - b.length) > 1) return false;
  let d = 0;
  for (let i = 0, j = 0; i < a.length || j < b.length; ) {
    if (a[i] === b[j]) {
      i++;
      j++;
      continue;
    }
    if (++d > 1) return false;
    if (a.length > b.length) i++;
    else if (b.length > a.length) j++;
    else {
      i++;
      j++;
    }
  }
  return true;
}

const params = new URLSearchParams(typeof location !== "undefined" ? location.search : "");
const scenario = params.get("mock") ?? "ready"; // ready | onboarding | indexing | empty

let settings: Settings = {
  folders: [
    { path: SS, kind: "screenshots", enabled: true },
    { path: PX, kind: "pictures", enabled: true },
    { path: DL, kind: "downloads", enabled: true },
  ],
  min_file_bytes: 2048,
  max_file_bytes: 40 * 1024 * 1024,
  min_dimension: 64,
  max_pixels: 80_000_000,
  hotkey: "Alt+Shift+S",
  semantic_enabled: true,
  paused: false,
  indexing_speed: "balanced",
  workers: 0,
  models_consented: scenario !== "onboarding",
};

const status = (): Status => {
  const indexing = scenario === "indexing";
  const total = indexing ? 5312 : 5312;
  const done = indexing ? 1240 : 5297;
  return {
    phase: settings.paused ? "paused" : indexing ? "indexing" : "idle",
    counts: { total, done, pending: indexing ? total - done - 9 : 0, failed: 2, skipped: 13 },
    current: indexing ? `${SS}\\Screenshot 2026-09-14 120533.png` : null,
    per_minute: indexing ? 212 : 0,
    ocr_available: scenario !== "onboarding",
    semantic_available: scenario !== "onboarding",
    watching: 3,
    last_error: null,
  };
};

export function mockApi(): Api {
  const statusCbs: ((s: Status) => void)[] = [];
  const progressCbs: ((p: DownloadProgress) => void)[] = [];
  const changedCbs: ((e: string | null) => void)[] = [];
  let installed = scenario !== "onboarding";
  const models = (): ModelsView => ({
    consented: settings.models_consented,
    downloading: false,
    runtime_error: null,
    models: [
      { id: "ocr", title: "Text reading (OCR)", detail: "PaddleOCR PP-OCRv4 text detection and recognition. Reads English, digits, punctuation and Chinese.", license: "Apache-2.0", homepage: "https://github.com/PaddlePaddle/PaddleOCR", required: true, download_bytes: 14_915_192, disk_bytes: 15_603_475, installed, pinned: true, sources: ["files.pythonhosted.org"] },
      { id: "clip", title: "Meaning search (CLIP)", detail: "OpenAI CLIP ViT-B/32, 8-bit quantized. Finds images by what they show: “cat meme”, “receipt”, “whiteboard”.", license: "MIT", homepage: "https://huggingface.co/Xenova/clip-vit-base-patch32", required: false, download_bytes: 153_600_000, disk_bytes: 153_600_000, installed, pinned: true, sources: ["huggingface.co"] },
    ],
  });
  const ok = async () => {};
  return {
    async search(q): Promise<SearchResults> {
      const t0 = performance.now();
      const qw = words(q.text);
      const f = q.filters;
      const allowed = (i: number) => {
        const h = hitOf(i, "recent", 0, null);
        if (f.screenshots_only && !h.is_screenshot) return false;
        if (f.folder && !h.path.startsWith(f.folder)) return false;
        if (f.from_ms && h.mtime_ms < f.from_ms) return false;
        if (f.to_ms && h.mtime_ms >= f.to_ms) return false;
        return true;
      };
      let hits: Hit[];
      if (!qw.length) {
        hits = ITEMS.map((_, i) => i).filter(allowed).map((i) => hitOf(i, "recent", 0, null));
      } else {
        hits = [];
        ITEMS.forEach((it, i) => {
          if (!allowed(i)) return;
          const tw = words(it.text + " " + it.name);
          const textHits = qw.filter((w) => tw.some((t) => near(w, t))).length / qw.length;
          const sem = qw.filter((w) => it.tags.some((t) => near(w, t))).length / qw.length;
          if (textHits === 0 && sem === 0) return;
          const kind = textHits > 0 && sem > 0 ? "both" : textHits > 0 ? "text" : "meaning";
          const score = (l: string) => qw.filter((w) => words(l).some((t) => near(w, t))).length;
          const line = it.text.split("\n").reduce<string | null>((best, l) => (score(l) > (best ? score(best) : 0) ? l : best), null);
          hits.push(hitOf(i, kind, textHits + 0.8 * sem, textHits > 0 ? line : null));
        });
        hits.sort((a, b) => b.score - a.score);
      }
      await new Promise((r) => setTimeout(r, 15));
      return { hits: hits.slice(0, q.limit), took_ms: performance.now() - t0 + 18, semantic_used: true, corrections: [] };
    },
    status: async () => status(),
    getSettings: async (): Promise<SettingsView> => ({ settings, defaults: settings.folders, data_dir: "C:\\Users\\Asha\\AppData\\Local\\io.github.shotfind" }),
    saveSettings: async (s) => {
      settings = s;
      statusCbs.forEach((cb) => cb(status()));
    },
    pause: async () => {
      settings.paused = true;
      statusCbs.forEach((cb) => cb(status()));
    },
    resume: async () => {
      settings.paused = false;
      statusCbs.forEach((cb) => cb(status()));
    },
    rescan: ok,
    retryFailed: async () => 2,
    wipeIndex: ok,
    models: async () => models(),
    downloadModels: async (ids: ModelId[]) => {
      settings.models_consented = true;
      const total = ids.includes("clip") ? 168_515_192 : 14_915_192;
      let done = 0;
      const tick = setInterval(() => {
        done = Math.min(total, done + total / 25);
        progressCbs.forEach((cb) => cb({ model: done < 14_915_192 ? "ocr" : "clip", file: "", downloaded: done, total, overall_downloaded: done, overall_total: total, stage: "downloading" }));
        if (done >= total) {
          clearInterval(tick);
          installed = true;
          changedCbs.forEach((cb) => cb(null));
        }
      }, 120);
    },
    cancelDownload: ok,
    importModels: async () => ["ocr"],
    skipModels: async () => {
      settings.models_consented = true;
    },
    open: ok,
    reveal: ok,
    copyImage: ok,
    copyText: async () => 42,
    details: async (id) => ({ file: hitOf(id - 1, "recent", 0, null), text: ITEMS[id - 1].text }),
    foldersInIndex: async () => [
      [SS, 4120],
      [PX, 806],
      [DL, 371],
    ],
    hideBar: ok,
    showMain: ok,
    appInfo: async () => ({ version: "0.1.0", os: "windows", onnxruntime: "C:\\Program Files\\shotfind\\onnxruntime\\onnxruntime.dll" }),
    pickFolder: async () => "C:\\Users\\Asha\\Desktop\\Receipts",
    thumbUrl: (id) => ITEMS[id - 1]?.img ?? "",
    fullUrl: (id) => ITEMS[id - 1]?.img ?? "",
    setWindowHeight: ok,
    onStatus: async (cb) => {
      statusCbs.push(cb);
      return () => {};
    },
    onModelProgress: async (cb) => {
      progressCbs.push(cb);
      return () => {};
    },
    onModelsChanged: async (cb) => {
      changedCbs.push(cb);
      return () => {};
    },
    onBarOpened: async () => () => {},
    onSetQuery: async () => () => {},
  };
}
