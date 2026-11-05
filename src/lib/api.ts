// Backend access. Inside Tauri this calls Rust commands; in a plain browser
// (vite dev, UI screenshots) it falls back to an in-memory mock.
import type {
  AppInfo,
  DownloadProgress,
  FileDetails,
  ModelId,
  ModelsView,
  Query,
  SearchResults,
  Settings,
  SettingsView,
  Status,
} from "./types";

export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

type Unlisten = () => void;

export interface Api {
  search(q: Query): Promise<SearchResults>;
  status(): Promise<Status>;
  getSettings(): Promise<SettingsView>;
  saveSettings(s: Settings): Promise<void>;
  pause(): Promise<void>;
  resume(): Promise<void>;
  rescan(): Promise<void>;
  retryFailed(): Promise<number>;
  wipeIndex(): Promise<void>;
  models(): Promise<ModelsView>;
  downloadModels(ids: ModelId[]): Promise<void>;
  cancelDownload(): Promise<void>;
  importModels(dir: string): Promise<ModelId[]>;
  skipModels(): Promise<void>;
  open(id: number): Promise<void>;
  reveal(id: number): Promise<void>;
  copyImage(id: number): Promise<void>;
  copyText(id: number): Promise<number>;
  details(id: number): Promise<FileDetails>;
  foldersInIndex(): Promise<[string, number][]>;
  hideBar(): Promise<void>;
  showMain(query?: string): Promise<void>;
  appInfo(): Promise<AppInfo>;
  pickFolder(): Promise<string | null>;
  thumbUrl(id: number): string;
  fullUrl(id: number): string;
  setWindowHeight(h: number): Promise<void>;
  onStatus(cb: (s: Status) => void): Promise<Unlisten>;
  onModelProgress(cb: (p: DownloadProgress) => void): Promise<Unlisten>;
  onModelsChanged(cb: (error: string | null) => void): Promise<Unlisten>;
  onBarOpened(cb: () => void): Promise<Unlisten>;
  onSetQuery(cb: (q: string) => void): Promise<Unlisten>;
}

async function tauriApi(): Promise<Api> {
  const { invoke } = await import("@tauri-apps/api/core");
  const { listen } = await import("@tauri-apps/api/event");
  const { getCurrentWindow, LogicalSize } = await import("@tauri-apps/api/window");
  const { open } = await import("@tauri-apps/plugin-dialog");
  const win = /Windows/i.test(navigator.userAgent);
  const base = win ? "http://thumb.localhost" : "thumb://localhost";
  const on = async <T,>(ev: string, cb: (p: T) => void) => listen<T>(ev, (e) => cb(e.payload));
  return {
    search: (query) => invoke("search", { query }),
    status: () => invoke("status"),
    getSettings: () => invoke("get_settings"),
    saveSettings: (settings) => invoke("save_settings", { settings }),
    pause: () => invoke("pause_indexing"),
    resume: () => invoke("resume_indexing"),
    rescan: () => invoke("rescan"),
    retryFailed: () => invoke("retry_failed"),
    wipeIndex: () => invoke("wipe_index"),
    models: () => invoke("models_status"),
    downloadModels: (ids) => invoke("download_models", { ids }),
    cancelDownload: () => invoke("cancel_download"),
    importModels: (dir) => invoke("import_models", { dir }),
    skipModels: () => invoke("skip_models"),
    open: (id) => invoke("open_file", { id }),
    reveal: (id) => invoke("reveal_file", { id }),
    copyImage: (id) => invoke("copy_image", { id }),
    copyText: (id) => invoke("copy_text", { id }),
    details: (id) => invoke("file_details", { id }),
    foldersInIndex: () => invoke("folders_in_index"),
    hideBar: () => invoke("hide_bar"),
    showMain: (query) => invoke("show_main_window", { query: query ?? null }),
    appInfo: () => invoke("app_info"),
    pickFolder: async () => {
      const r = await open({ directory: true, multiple: false });
      return typeof r === "string" ? r : null;
    },
    thumbUrl: (id) => `${base}/${id}`,
    fullUrl: (id) => `${base}/full/${id}`,
    setWindowHeight: (h) => getCurrentWindow().setSize(new LogicalSize(760, h)),
    onStatus: (cb) => on("index-status", cb),
    onModelProgress: (cb) => on("model-progress", cb),
    onModelsChanged: (cb) => on("models-changed", cb),
    onBarOpened: (cb) => on("bar-opened", () => cb()),
    onSetQuery: (cb) => on("set-query", cb),
  };
}

let apiPromise: Promise<Api> | null = null;

export function getApi(): Promise<Api> {
  if (!apiPromise) {
    apiPromise = isTauri ? tauriApi() : import("./mock").then((m) => m.mockApi());
  }
  return apiPromise;
}
