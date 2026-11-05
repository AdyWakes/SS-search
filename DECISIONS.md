# Decisions log

Ambiguities resolved during the autonomous build, newest last. Each entry: what, why, how to revisit.

1. **Name: kept "shotfind".** `shotfind` is unused on crates.io and npm (checked 2026-11-05). It is short, says what it does, and is easy to type. Bundle identifier: `io.github.shotfind`.
2. **Workspace split.** All indexing/search logic lives in `crates/shotfind-core` (no Tauri dependency), so it can be tested on any CI runner without WebKit/GTK. `src-tauri` is a thin shell: commands, windows, hotkey, tray.
3. **ONNX Runtime is loaded dynamically (`ort` `load-dynamic`) and bundled as a Tauri resource.** `ort`'s default `download-binaries` fetches from cdn.pyke.io at build time, which was unreachable from the build sandbox and adds a third-party CDN to the supply chain. Instead `scripts/fetch-onnxruntime.mjs` downloads Microsoft's official release (v1.22.0) from GitHub, verifies a pinned SHA-256, and drops the shared library in `src-tauri/resources/onnxruntime/`. The app loads it by absolute path, which also avoids picking up the old `onnxruntime.dll` that ships in `C:\Windows\System32` on some Windows builds.
4. **`ort` pinned to `=2.0.0-rc.15`.** It is a release candidate (no 2.0 stable yet); pinning exactly protects against API churn between RCs. Its baseline C API level works with ONNX Runtime >= 1.17.
5. **HEIC/HEIF/AVIF not decoded in v0.1.** Decoding needs libheif (LGPL, native, painful to ship on Windows) or OS codecs (WIC on Windows needs the paid/optional HEVC extension). Files with these extensions are recognised but not indexed. Tracked as a "good first issue" (platform decoders: WIC on Windows, ImageIO on macOS).
6. **Indexing order: screenshots first, newest first.** People search for recent screenshots; this makes the app useful within the first minute of indexing.
7. **Crash safety model.** Each file's attempt counter is committed *before* processing and reset on success. On startup, files with >= 3 unfinished attempts are marked failed ("crashed repeatedly") so one poisoned file can't crash-loop the app. Panics inside engines are caught per file. All per-file results are written in one SQLite transaction (WAL mode).
8. **Skipped folders:** hidden dirs (`.*`), `node_modules`, recycle bins, `*.photoslibrary` / Lightroom `*.lrdata` (derived copies of photos, huge), app bundles, and the app's own data dir. Nested enabled folders (Pictures/Screenshots inside Pictures) are scanned once, as their own root.
9. **Defaults for skip rules:** min file 2 KB, max 40 MB, min side 64 px (icons), max 80 MP (decompression-bomb guard). All configurable in Settings.
10. **Thumbnails:** 384 px longest side, JPEG q78 (~15–30 KB each), sharded `thumbs/xx/<id>.jpg` in app data.
11. **Background priority:** coordinator and worker threads use `THREAD_MODE_BACKGROUND_BEGIN` on Windows (lowers CPU *and* I/O priority) and lowest priority elsewhere. Default workers = cores/4 clamped to 1..3.
