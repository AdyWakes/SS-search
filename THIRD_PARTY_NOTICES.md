# Third-party notices

shotfind bundles or downloads the following. Their licenses apply to those parts.

| Component | How it is used | License |
|---|---|---|
| [ONNX Runtime](https://github.com/microsoft/onnxruntime) 1.22.0 | Bundled shared library (from Microsoft's official release) | MIT; see `ONNXRUNTIME-LICENSE.txt` and `ONNXRUNTIME-ThirdPartyNotices.txt` shipped next to it |
| [PaddleOCR](https://github.com/PaddlePaddle/PaddleOCR) PP-OCRv4 det/rec models | Downloaded on first run (ONNX export distributed in the [RapidOCR](https://github.com/RapidAI/RapidOCR) PyPI wheel) | Apache-2.0 |
| [OpenAI CLIP](https://github.com/openai/CLIP) ViT-B/32 | Downloaded on first run (8-bit ONNX export by [Xenova](https://huggingface.co/Xenova/clip-vit-base-patch32)) | MIT |
| CLIP BPE merges (`bpe_simple_vocab_16e6.txt.gz`) | Compiled into the binary | MIT (openai/CLIP) |
| DejaVu Sans | Test fixtures only (not shipped) | Bitstream Vera / DejaVu license |

Rust and npm dependencies are listed in `Cargo.lock` and `package-lock.json` with their own licenses (MIT/Apache-2.0 for the large majority).
