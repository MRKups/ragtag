# Ragtag Implementation Tracker

*Active work only. Completed and superseded entries are removed; architectural decisions remain documented in `docs/`.*

## Active items

- [ ] **B185 — Bound PDF stream decompression before parser allocation.** Current source-size, page-count, and retained-text caps do not constrain `pdf-extract`/`lopdf` inflation of compressed content or object streams, because those dependencies allocate decoded `Vec<u8>` values before Ragtag's output writer runs. `lopdf`’s upstream bounded API still has an open object-stream error-propagation and PNG-predictor allocation gap ([J-F-Liu/lopdf#569](https://github.com/J-F-Liu/lopdf/issues/569)). Replace or constrain the extraction path with a decoder-enforced cumulative allocation budget; do not claim compressed-PDF bomb protection until then.
