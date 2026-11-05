# Progress

Resume guide for a new session: read this, then `DECISIONS.md`.

## Phases
- [x] 1. Scaffold + CI
- [ ] 2. Scanner + watcher + SQLite schema
- [ ] 3. OCR pipeline
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
Phase 2.

## Known issues
- (none yet)
