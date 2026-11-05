# Good morning — shotfind status

Branch: `claude/busy-noether-d0hfd6` (all 7 phases plus the self-review are committed and pushed; there is no PR, release or deploy).

## Summary

**shotfind** (I kept the name: it's free on crates.io and npm) is a Tauri v2 + React app that makes screenshots searchable offline:

- **OCR:** PaddleOCR PP-OCRv4 through ONNX Runtime, with tiling for tall captures and fixes for digit look-alikes.
- **Meaning search:** CLIP ViT-B/32 (int8), with a Rust port of CLIP's tokenizer.
- **Search:** SQLite FTS5 with typo tolerance plus an int8 vector index, combined into one ranking. Filters for folder, date range and screenshots only.
- **Indexing:** background, low priority, resumable, and crash-safe (it quarantines files that crash it). A file watcher picks up new screenshots. You can pause and resume.
- **UI:** big search field and thumbnail grid, preview with the extracted text, settings, a first-run consent screen, and the **Alt+Shift+S** quick bar. Plus a tray icon, open / show in folder / copy image / copy text, and light/dark following the OS.
- **Privacy:** models download once after consent, checked against pinned SHA-256s. There are no other network calls. User files are opened read-only, which is enforced by a static audit test and a behavioural one.
- **CI:** lint + tests on Linux/Windows/macOS, plus a real-model job. Pushing a tag builds Windows .msi/.exe, macOS .dmg (arm64 + x64) and Linux .AppImage/.deb into a **draft** release.

Docs: `README.md` (user-facing), `DECISIONS.md` (37 logged decisions and why), `PROGRESS.md` (resume guide), `THIRD_PARTY_NOTICES.md`, and UI screenshots in `docs/screenshots/`.

## ⚠️ Read this first: model hashes

Hugging Face was blocked from my sandbox, so I couldn't hash the CLIP vision model myself. Instead, CI's `model-tests` job downloaded it on a clean GitHub runner and printed its SHA-256, and I pinned that value (DECISIONS #36). The same CI run independently confirmed the CLIP text hash I had found via web search. With the hash pinned, the app's own downloader installed real CLIP in CI and the meaning-search ranking test passed.

To double-check the pinned hashes against upstream yourself (needs internet):

```powershell
node scripts/pin-models.mjs --verify
```

## What works (verified here)

- **49 tests pass on Linux** (`cargo test -p shotfind-core`), plus 6 ignored real-model and benchmark tests. They cover:
  - indexing: skip rules, incremental rescan, the watcher, resume after a crash, quarantine, pause and restart, models installed later
  - search ranking: exact, typos, OCR look-alikes, numbers, prefixes, meaning, combined, filters, snippets, odd FTS input
  - the never-modify guarantee: a read-only tree stays byte/mtime/permission-identical, plus a static audit of the source
  - the tokenizer, the downloader's hash rejection, and recovery from a corrupt database
- **Real-model tests pass in CI:** OCR on rendered text and on a 60-line scrolling capture. Real CLIP is installed by the app's downloader and ranks a chart, a sunset and an invoice correctly for matching queries. The CLIP plumbing is also tested with hand-built ONNX models.
- **The real app ran on Linux (Xvfb).** The bundled ONNX Runtime loaded, 4 screenshots were OCR'd, and thumbnails displayed. The typo query `upl paymnet rahul` found the right screenshots in 4 ms. Alt+Shift+S opened the quick bar, and Ctrl+Shift+C put the OCR text on the clipboard. Captures: `docs/screenshots/real-app-linux-*.png`.
- **The real downloader works:** it installs OCR (PyPI wheel → verified → extracted → verified) and CLIP (direct files → verified). CI caught a bug in the CLIP path, where the model folder wasn't created; that's fixed.
- **Windows compile check:** `cargo check --target x86_64-pc-windows-gnu` passes for both crates. Early CI runs failed on a Windows-only API mismatch and on new clippy lints; both are fixed. The latest Windows test run had one possibly flaky failure: 5 indexed files instead of 4 right after *Delete index*, in a test that had passed on Windows in the previous run. That assertion now dumps every indexed row when it fails, so the next failure will show the cause.
- **Performance:**
  - search, worst case: 84 ms on a 20k-image synthetic index
  - scan: 10k files in 0.2 s
  - real OCR indexing on this 4-vCPU VM: 98 files/min (Balanced) and 132 files/min (Fast) on dense screenshots
  - memory: about 280 MB peak per OCR worker, and models unload when idle

## What's weak / not done

1. **Meaning-search tuning is rough.** With real CLIP, matching pairs scored only 0.233–0.257 and non-matching ones 0.18–0.237 on synthetic test images, so the margins are thin. I've set `search::tuning` from those numbers. On your real library, try `cat`, `receipt`, `chart` and `whiteboard`, and adjust `SEM_MIN`/`SEM_WINDOW` if results look too loose or too strict.
2. **Never run on real Windows or macOS.** It compiles for Windows, and CI runs the core tests on all three OSes, but the GUI was only exercised on Linux. Things to watch on Windows:
   - tray behaviour
   - quick-bar focus and blur
   - `http://thumb.localhost` image loading
   - copying an image to the clipboard
   - the global shortcut when another app already owns it (there's no UI error yet; it's only logged)
3. **Indexing speed is "a few minutes" only on a decent machine.** 1,000 dense screenshots take about 7–10 minutes on 4 vCPUs. Ideas: skip identical files by content hash, or start in *Fast* until the first pass is done.
4. **Gaps:** HEIC/AVIF aren't decoded. Chinese/Japanese words aren't segmented. Side-by-side columns get interleaved in "copy text" output.
5. **Platform polish:** builds are unsigned (SmartScreen/Gatekeeper warnings). There's no "start at login" toggle (`--hidden` exists but nothing registers it). Global shortcuts don't work on pure Wayland.
6. **Minor:**
   - The quick bar is a plain frameless window (no rounded or transparent corners).
   - The thumbnail cache grows about 20 KB per image.
   - Reading files may update access times on filesystems with atime enabled. Contents, mtime and permissions are never changed.

## Test on Windows: exact steps

**1. Prerequisites (once).**

- Git: https://git-scm.com
- Node.js 22 LTS: https://nodejs.org
- Rust: https://rustup.rs. Choose the default MSVC toolchain. When asked, install **Visual Studio Build Tools** with the "Desktop development with C++" workload.
- WebView2 is preinstalled on Windows 11. On Windows 10 the installer bundles it.

**2. Get the code** (PowerShell):

```powershell
git clone https://github.com/AdyWakes/SS-search.git
cd SS-search
git checkout claude/busy-noether-d0hfd6
```

**3. (Optional) Re-verify the pinned model hashes:**

```powershell
node scripts/pin-models.mjs --verify
```

**4. Install and fetch the bundled ONNX Runtime:**

```powershell
npm install
node scripts/fetch-onnxruntime.mjs
```

**5. Run the tests** (about 3 minutes on a cold build):

```powershell
cargo test -p shotfind-core
```

**6. Run the app in dev mode.** The first compile takes 5–10 minutes. Set `RUST_LOG=debug` in the same terminal first if you want verbose logs.

```powershell
npm run tauri dev
```

**7. Click through this checklist:**

- [ ] The first-run screen lists two models with sizes and sources. Click **Download** and watch the progress bar.
- [ ] The footer shows "Indexing N of M". The thin orange line under the header tracks progress.
- [ ] Press **Win+PrtScn**. The new screenshot appears under *Recent* within about 5 seconds.
- [ ] Search for words you know are in a screenshot. Then misspell one word (e.g. `paymnet`). Then search a number like `404`.
- [ ] Search by content: `cat`, `receipt`, `chart`.
- [ ] Filters: *Screenshots only*, *Past 7 days*, and the folder dropdown.
- [ ] Keyboard: arrow keys to select, **Enter** opens, **Space** previews, **Ctrl+Enter** shows in Explorer, **Ctrl+C** copies the image (paste into Paint), **Ctrl+Shift+C** copies the text (paste into Notepad).
- [ ] Switch to another app and press **Alt+Shift+S**. The quick bar opens. Type, press **Enter** or **Esc**, and press **Tab** to open all results.
- [ ] Close the main window: shotfind stays in the tray and indexing continues. Use tray → **Quit** to exit.
- [ ] Settings: add a folder, toggle one off, change speed to *Fast*, pause and resume, record a new shortcut, and **Delete index…** (it rebuilds).
- [ ] Restart the app in the middle of indexing. It continues where it stopped.

**8. Optional proof that your files aren't touched.** Hash a folder before and after a full index and a wipe, then compare. There should be no output.

```powershell
Get-ChildItem $env:USERPROFILE\Pictures\Screenshots -Recurse -File | Get-FileHash | Export-Csv before.csv
```

```powershell
Get-ChildItem $env:USERPROFILE\Pictures\Screenshots -Recurse -File | Get-FileHash | Export-Csv after.csv
```

```powershell
Compare-Object (Import-Csv before.csv) (Import-Csv after.csv) -Property Hash, Path
```

**9. Build the installers.** Output goes to `target\release\bundle\msi\` and `target\release\bundle\nsis\`.

```powershell
npm run tauri build
```

**10. Where things live:**

- Index, thumbnails and models: `%LOCALAPPDATA%\io.github.shotfind`. Delete the folder for a clean first run.
- Settings: `%LOCALAPPDATA%\io.github.shotfind\settings.json`.

## To ship a preview release later

Pin the hash, commit, then `git tag v0.1.0 && git push origin v0.1.0`. The release workflow refuses to build if any model hash is unpinned, and it produces a **draft** release you can test before publishing. I didn't create any tags or releases.
