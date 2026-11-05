// Mirrors the Rust types serialised by the backend.

export interface FileRow {
  id: number;
  path: string;
  dir: string;
  name: string;
  size: number;
  mtime_ms: number;
  width: number | null;
  height: number | null;
  is_screenshot: boolean;
  state: number;
}

export type MatchKind = "recent" | "text" | "name" | "meaning" | "both";

export interface Hit extends FileRow {
  score: number;
  text_score: number;
  semantic_score: number;
  kind: MatchKind;
  snippet: string | null;
}

export interface SearchResults {
  hits: Hit[];
  took_ms: number;
  semantic_used: boolean;
  corrections: [string, string][];
}

export interface Filters {
  folder?: string | null;
  from_ms?: number | null;
  to_ms?: number | null;
  screenshots_only: boolean;
}

export interface Query {
  text: string;
  filters: Filters;
  limit: number;
}

export type Phase = "starting" | "scanning" | "indexing" | "idle" | "paused" | "stopped";

export interface Counts {
  total: number;
  done: number;
  pending: number;
  failed: number;
  skipped: number;
}

export interface Status {
  phase: Phase;
  counts: Counts;
  current: string | null;
  per_minute: number;
  ocr_available: boolean;
  semantic_available: boolean;
  watching: number;
  last_error: string | null;
}

export type FolderKind = "screenshots" | "pictures" | "downloads" | "desktop" | "custom";

export interface Folder {
  path: string;
  kind: FolderKind;
  enabled: boolean;
}

export type IndexingSpeed = "gentle" | "balanced" | "fast";

export interface Settings {
  folders: Folder[];
  min_file_bytes: number;
  max_file_bytes: number;
  min_dimension: number;
  max_pixels: number;
  hotkey: string;
  semantic_enabled: boolean;
  paused: boolean;
  indexing_speed: IndexingSpeed;
  workers: number;
  models_consented: boolean;
}

export interface SettingsView {
  settings: Settings;
  defaults: Folder[];
  data_dir: string;
}

export type ModelId = "ocr" | "clip";

export interface ModelStatus {
  id: ModelId;
  title: string;
  detail: string;
  license: string;
  homepage: string;
  required: boolean;
  download_bytes: number;
  disk_bytes: number;
  installed: boolean;
  pinned: boolean;
  sources: string[];
}

export interface ModelsView {
  models: ModelStatus[];
  consented: boolean;
  downloading: boolean;
  runtime_error: string | null;
}

export interface DownloadProgress {
  model: ModelId;
  file: string;
  downloaded: number;
  total: number;
  overall_downloaded: number;
  overall_total: number;
  stage: "downloading" | "extracting";
}

export interface FileDetails {
  file: FileRow;
  text: string;
}

export interface AppInfo {
  version: string;
  os: string;
  onnxruntime: string | null;
}
