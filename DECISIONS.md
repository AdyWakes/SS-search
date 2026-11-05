# Decisions log

Ambiguities resolved during the autonomous build, newest last. Each entry: what, why, how to revisit.

1. **Name: kept "shotfind".** `shotfind` is unused on crates.io and npm (checked 2026-11-05). It is short, says what it does, and is easy to type. Bundle identifier: `io.github.shotfind`.
2. **Workspace split.** All indexing/search logic lives in `crates/shotfind-core` (no Tauri dependency), so it can be tested on any CI runner without WebKit/GTK. `src-tauri` is a thin shell: commands, windows, hotkey, tray.
3. **ONNX Runtime is loaded dynamically (`ort` `load-dynamic`) and bundled as a Tauri resource.** `ort`'s default `download-binaries` fetches from cdn.pyke.io at build time, which was unreachable from the build sandbox and adds a third-party CDN to the supply chain. Instead `scripts/fetch-onnxruntime.mjs` downloads Microsoft's official release (v1.22.0) from GitHub, verifies a pinned SHA-256, and drops the shared library in `src-tauri/resources/onnxruntime/`. The app loads it by absolute path, which also avoids picking up the old `onnxruntime.dll` that ships in `C:\Windows\System32` on some Windows builds.
4. **`ort` pinned to `=2.0.0-rc.15`.** It is a release candidate (no 2.0 stable yet); pinning exactly protects against API churn between RCs. Its baseline C API level works with ONNX Runtime >= 1.17.
