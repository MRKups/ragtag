# Third-Party Notices & Licenses

Ragtag is released under the **MIT License**. This application incorporates and depends upon several open-source libraries and frameworks. We gratefully acknowledge the creators and maintainers of these projects.

A complete, machine-readable inventory of all 1,196 direct and transitive dependencies and their declared licenses is maintained in [`docs/third-party-license-inventory.json`](docs/third-party-license-inventory.json).

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
- **License**: [MIT License](https://github.com/tauri-apps/tauri/blob/dev/LICENSE_MIT) or [Apache-2.0 License](https://github.com/tauri-apps/tauri/blob/dev/LICENSE_APACHE)

### 3. Vector Storage & Search
- **Component**: [LanceDB](https://github.com/lancedb/lancedb) & [Apache Arrow](https://github.com/apache/arrow-rs)
- **Copyright**: © 2023 LanceDB Inc., © The Apache Software Foundation
- **License**: [Apache-2.0 License](https://www.apache.org/licenses/LICENSE-2.0)

### 4. Local Embedding Generation
- **Component**: [FastEmbed-rs](https://github.com/Anush008/fastembed-rs) & [ONNX Runtime](https://github.com/microsoft/onnxruntime)
- **Copyright**: © 2023 Anush Shetty, © Microsoft Corporation
- **License**: [MIT License](https://github.com/Anush008/fastembed-rs/blob/main/LICENSE) / [Apache-2.0 License](https://github.com/microsoft/onnxruntime/blob/main/LICENSE)

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
- **Marked**: © 2011 - 2024 Christopher Jeffrey ([MIT](https://github.com/markedjs/marked/blob/master/LICENSE.md))
- **Lucide Icons**: © 2022 - Present Lucide Contributors ([ISC](https://github.com/lucide-icons/lucide/blob/main/LICENSE))
- **Vite**: © 2019 - Present Evan You & Vite Contributors ([MIT](https://github.com/vitejs/vite/blob/main/LICENSE))

---

## Machine Learning Models & Weights Notice

The MIT license covering this repository applies solely to the software source code and configuration. Machine learning model checkpoints, tokenizer definitions, and weights downloaded by users at runtime (such as EmbeddingGemma and community GGUF models) are separate works governed by their respective upstream licenses (e.g., Google Gemma Terms of Use, Meta Llama Community License, or Apache-2.0). Users are responsible for complying with any applicable model-specific license terms.
