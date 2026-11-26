# Third-Party Notices & Licenses

Ragtag is released under the **MIT License**. This application incorporates and depends upon several open-source libraries and frameworks. We gratefully acknowledge the creators and maintainers of these projects.

Declared dependency licenses for the default-feature Cargo resolution and frontend lockfile are listed in [`docs/third-party-license-inventory.json`](docs/third-party-license-inventory.json). Disabled optional Rust features such as CUDA need a separate inventory review. Update the inventory with `npm run licenses:generate` after dependency changes.

---

## Key Open-Source Components

### 1. In-Process Inference Engine
- **Component**: [`mistral.rs`](https://github.com/EricLBuehler/mistral.rs) / `mistralrs-core`
- **Copyright**: © 2024 Eric Buehler
- **License**: [MIT License](https://github.com/EricLBuehler/mistral.rs/blob/master/LICENSE)
- **Notice**: A vendored copy of `mistralrs-core` is maintained in `src-tauri/vendor/mistralrs-core/` with a minor concurrency projection patch to allow batched quantized projection without global lock acquisition. Original copyright and license notices are retained.

### 2. Desktop Application Shell
- **Component**: [Tauri v2](https://github.com/tauri-apps/tauri)
- **Copyright**: © 2019 - Present Tauri Programme within The Commons Conservancy
- **License**: [MIT License](https://github.com/tauri-apps/tauri/blob/dev/LICENSE-MIT) or [Apache-2.0 License](https://github.com/tauri-apps/tauri/blob/dev/LICENSE-APACHE-2.0)

### 3. Vector Storage & Search
- **Component**: [LanceDB](https://github.com/lancedb/lancedb) & [Apache Arrow](https://github.com/apache/arrow-rs)
- **Copyright**: © 2023 LanceDB Inc., © The Apache Software Foundation
- **License**: [Apache-2.0 License](https://www.apache.org/licenses/LICENSE-2.0)

### 4. Local Embedding Generation
- **Component**: [FastEmbed-rs](https://github.com/Anush008/fastembed-rs) & [ONNX Runtime](https://onnxruntime.ai/)
- **Copyright**: © 2023 Anush Shetty, © Microsoft Corporation
- **License**: FastEmbed [Apache-2.0](https://github.com/Anush008/fastembed-rs/blob/main/LICENSE) / ONNX Runtime MIT ([upstream package metadata](https://pypi.org/project/onnxruntime/); its GitHub source/license links were unavailable when reviewed on 2026-11-26).

### 5. Embedded Relational Database
- **Component**: [SQLx](https://github.com/launchbadge/sqlx) & [SQLite](https://www.sqlite.org/)
- **Copyright**: © 2020 LaunchBadge, LLC. SQLite is in the Public Domain.
- **License**: [MIT License](https://github.com/launchbadge/sqlx/blob/main/LICENSE-MIT) or [Apache-2.0 License](https://github.com/launchbadge/sqlx/blob/main/LICENSE-APACHE)

### 6. Document Parsing & Extraction
- **Components**: [pdf-extract](https://github.com/jrmuizel/pdf-extract), [lopdf](https://github.com/J-F-Liu/lopdf), [quick-xml](https://github.com/tafia/quick-xml), [zip](https://github.com/zip-rs/zip2), [pulldown-cmark](https://github.com/raphlinus/pulldown-cmark)
- **Licenses**: MIT or Apache-2.0

### 7. Frontend User Interface & Rendering
- **Svelte 5**: © 2016 - Present Rich Harris and Svelte contributors ([MIT](https://github.com/sveltejs/svelte/blob/main/LICENSE.md))
- **PDF.js (`pdfjs-dist`)**: © Mozilla Foundation ([Apache-2.0](https://github.com/mozilla/pdf.js/blob/master/LICENSE))
- **DOMPurify**: © 2015 Cure53 ([MPL-2.0 / Apache-2.0](https://github.com/cure53/DOMPurify/blob/main/LICENSE))
- **Marked**: © 2011 - 2024 Christopher Jeffrey ([MIT](https://github.com/markedjs/marked/blob/master/LICENSE))
- **Lucide Icons**: © 2022 - Present Lucide Contributors ([ISC](https://github.com/lucide-icons/lucide/blob/main/LICENSE))
- **Vite**: © 2019 - Present Evan You & Vite Contributors ([MIT](https://github.com/vitejs/vite/blob/main/LICENSE))

---

## Machine Learning Models & Weights Notice

The MIT license covering this repository applies solely to first-party software source code and configuration. Model checkpoints, tokenizer definitions, and weights downloaded at runtime are separate works governed by their upstream terms. No model weights are included in this source repository.

| Default runtime asset | Upstream terms |
| --- | --- |
| [EmbeddingGemma ONNX](https://huggingface.co/onnx-community/embeddinggemma-300m-ONNX) | [Gemma Terms of Use](https://ai.google.dev/gemma/terms) |
| [Qwen3-1.7B GGUF](https://huggingface.co/unsloth/Qwen3-1.7B-GGUF) | [Apache-2.0](https://www.apache.org/licenses/LICENSE-2.0) |
| [Qwen3-1.7B tokenizer assets](https://huggingface.co/Qwen/Qwen3-1.7B) | [Apache-2.0](https://www.apache.org/licenses/LICENSE-2.0) |

These are the defaults reviewed on 2026-11-26. Users selecting other models must review those models' terms separately.

## Screenshot Content

`assets/screenshots/query-citations.webp` displays excerpts and a PDF rendering of [“United Nations” on English Wikipedia](https://en.wikipedia.org/wiki/United_Nations), by [Wikipedia contributors](https://en.wikipedia.org/w/index.php?title=United_Nations&action=history). The reproduced article text is available under [Creative Commons Attribution-ShareAlike 4.0](https://creativecommons.org/licenses/by-sa/4.0/), not the repository's MIT software license. The article is reformatted as a PDF, excerpted, and displayed alongside a generated answer.

Article illustrations retain their individual terms:

- [Membership map](https://commons.wikimedia.org/wiki/File:United_Nations_%28Member_States_and_Territories%29.svg), by Newfraferz87 and subsequent Commons contributors: [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/), reduced in the PDF preview.
- [UN flag](https://commons.wikimedia.org/wiki/File:Flag_of_the_United_Nations.svg) and [UN emblem](https://commons.wikimedia.org/wiki/File:UN_emblem_blue.svg): public-domain design/rendering notices and separate insignia restrictions are recorded on their file pages.
- [League of Nations photograph](https://commons.wikimedia.org/wiki/File:Emperor_Haile_Selassie_League_of_Nations_speech.png), Bettmann Archive: Commons records public-domain status in the United States; its file page explains jurisdictional limitations.

Ragtag's contributions to this composite query screenshot are also available under [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/). This does not relicense the software or override the individual terms of third-party material.

Wikipedia/Wikimedia and United Nations names, logos, and emblems remain subject to their respective trademark and emblem-use rules. Their appearance identifies the example source; it does not imply endorsement of Ragtag.
