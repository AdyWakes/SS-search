# Progress

Resume guide for a new session: read this, then `DECISIONS.md`.

## Phases
- [x] 1. Scaffold + CI
- [x] 2. Scanner + watcher + SQLite schema
- [x] 3. OCR pipeline
- [x] 4. Embeddings pipeline + model download flow
- [x] 5. Search + ranking + tests
- [x] 6. UI: search grid, hotkey bar, settings, indexing progress
- [ ] 7. Release workflow + README
- [ ] Self-review, fixes, UI screenshots, MORNING.md

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
Phase 7 (release workflow + README), then self-review.

## Known issues
- Ranking constants (`search::tuning`) are calibrated on fake embeddings only; tune with real CLIP once pinned.
- Search latency: 20k-file synthetic index, worst query 84 ms (release build, this VM).
- Chinese/Japanese text is indexed, but FTS `unicode61` does not segment CJK words, so CJK search only matches whole runs of characters.
- **CLIP vision model SHA-256 not pinned** (Hugging Face blocked in the build sandbox). Meaning search stays off until `node scripts/pin-models.mjs` is run and the manifest committed.
- OCR speed (this VM, 1 thread): ~0.35 s detection + ~0.03–0.05 s per text line for a dense 1080p screenshot (~0.9 s total). With 2 workers x 2 threads, about 0.25–0.4 s per screenshot wall time.
- Reading order interleaves side-by-side columns (search is unaffected; "copy text" output can mix columns).
