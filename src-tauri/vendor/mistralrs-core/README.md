# `mistralrs-core`

Core crate of `mistral.rs` including the models and associated executors.

Documentation: https://docs.rs/mistralrs-core/

## Ragtag vendored patch

Upstream: [`mistral.rs` v0.9.5, revision `465191d0b0a7e2432d327b0de06204ab3f94d3a2`](https://github.com/EricLBuehler/mistral.rs/tree/465191d0b0a7e2432d327b0de06204ab3f94d3a2/mistralrs-core).

Patch revision: `ragtag.1`. In `src/pipeline/mod.rs`, narrowed batched decode logits are made contiguous before quantized projection. The regression `pipeline::tests::batched_logits_support_quantized_projection` covers this repair without serializing concurrent requests.

The manifest resolves upstream workspace inheritance and pins sibling Git dependencies to the same revision. The build script reports the upstream revision plus `ragtag.1` rather than Ragtag's Git HEAD. The upstream MIT license is retained in `LICENSE`; all other upstream source is unchanged.

Run the focused regression from Ragtag's `src-tauri/` directory:

```sh
cargo test --locked -p mistralrs-core --lib pipeline::tests::batched_logits_support_quantized_projection -- --exact
```

See [Ragtag's decision record](../../../docs/decisions.md#d007--local-in-process-inference-by-default) for the maintenance and runtime qualification boundary.
