# Progress

Resume guide for a new session: read this, then `DECISIONS.md`.

## Phases
- [x] 1. Scaffold + CI
- [x] 2. Scanner + watcher + SQLite schema
- [x] 3. OCR pipeline
- [x] 4. Embeddings pipeline + model download flow
- [x] 5. Search + ranking + tests
- [x] 6. UI: search grid, hotkey bar, settings, indexing progress
- [x] 7. Release workflow + README
- [x] Self-review, fixes, UI screenshots
- [x] MORNING.md

## Verified in the sandbox (Linux, Xvfb)
- Real app: launched, ONNX Runtime loaded from bundled resources, 4 test images scanned + OCR'd, thumbnails via `thumb://`, typo query `upl paymnet rahul` found the chat screenshots in 4 ms, global hotkey Alt+Shift+S opened the quick bar, Ctrl+Shift+C put OCR text on the clipboard.
- UI against the mock backend: `node scripts/ui-screenshots.mjs`.

## How to build locally
```bash
npm install
node scripts/fetch-onnxruntime.mjs      # once; puts ONNX Runtime into src-tauri/resources/onnxruntime
npm run tauri dev
cargo test -p shotfind-core            # core tests, no models required
```

## Next
Pin the CLIP vision hash, calibrate `search::tuning` with real CLIP, test on real Windows.

## Known issues
- Ranking constants (`search::tuning`) are calibrated on fake embeddings only; tune with real CLIP once pinned.
- Search latency: 20k-file synthetic index, worst query 84 ms (release build, this VM).
- Chinese/Japanese text is indexed, but FTS `unicode61` does not segment CJK words, so CJK search only matches whole runs of characters.
- **CLIP vision model SHA-256 not pinned** (Hugging Face blocked in the build sandbox). Meaning search stays off until `node scripts/pin-models.mjs` is run and the manifest committed.
- Indexing speed (this 4-vCPU VM, real OCR, dense screenshots): 98 files/min Balanced, 132 files/min Fast. 1,000 dense screenshots = 7–10 min here; ~3–5 min expected on an 8-thread laptop.
- Reading order interleaves side-by-side columns (search is unaffected; "copy text" output can mix columns).
