---
title: Documentation development process
normative-for: [documentation maintenance]
depends: [documentation-style.md]
status: current guide. Source code and migrations remain authoritative.
---

# Documentation development process

Maintain the document that owns a changed contract in the same change as the implementation.

## Choose the document

- Architecture owns subsystem boundaries, flow outcomes, and degradation behavior.
- Data schema owns persisted data, settings, and IPC wire vocabulary.
- Decisions own durable choices and their tradeoffs.
- File structure owns repository navigation.
- The tracker owns unresolved work.
- Release readiness owns the current publication gate.
- Generated records own generated facts.

Use a cross-reference when another document already owns the concept.

## Write and review

1. Read the affected current document before changing the implementation.
2. Verify each changed factual claim against source code, migrations, generated output, or the final release artifact.
3. Use `title`, `normative-for`, `depends`, and `status` front matter for new current Markdown documents. Reuse those keys rather than adding document-specific metadata conventions.
4. Keep implementation mechanics in source. Document observable behavior, authority boundaries, inputs, outputs, limits, and recovery behavior.
5. Validate relative links, generated-record syntax, and the changed facts before merge.

## Preserve record states

Current documents describe the present system. The tracker describes only unresolved work. Generated records identify their inputs and regeneration method. Dated review records are archival evidence: retain their findings and validation context, add a current successor when known, and do not rewrite them to represent later source.
