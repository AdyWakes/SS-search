# Progress

Resume guide for a new session: read this, then `DECISIONS.md`.

## Phases
- [x] 1. Scaffold + CI
- [x] 2. Scanner + watcher + SQLite schema
- [x] 3. OCR pipeline
- [ ] 4. Embeddings pipeline + model download flow
- [ ] 5. Search + ranking + tests
- [ ] 6. UI: search grid, hotkey bar, settings, indexing progress
- [ ] 7. Release workflow + README
- [ ] Self-review, fixes, UI screenshots, MORNING.md

## How to build locally
```bash
npm install
node scripts/fetch-onnxruntime.mjs      # once; puts ONNX Runtime into src-tauri/resources/onnxruntime
npm run tauri dev
cargo test -p shotfind-core            # core tests, no models required
```

## Next
Phase 4 (embeddings + model download).

## Known issues
- OCR speed (this VM, 1 thread): ~0.35 s detection + ~0.03–0.05 s per text line for a dense 1080p screenshot (~0.9 s total). With 2 workers x 2 threads, about 0.25–0.4 s per screenshot wall time.
- Reading order interleaves side-by-side columns (search is unaffected; "copy text" output can mix columns).
