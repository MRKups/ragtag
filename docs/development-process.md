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

Cross-reference the document that already owns a concept.

## Write and review

1. Read the affected current document before implementation.
2. Verify changed facts against source code, migrations, generated output, or the release artifact.
3. New current Markdown documents use `title`, `normative-for`, `depends`, and `status`.
4. Keep mechanics in source. Document behavior, authority, inputs, outputs, limits, and recovery.
5. Validate relative links, generated-record syntax, and changed facts before merge.

## Preserve record states

Current documents describe the present system. The tracker describes only unresolved work. Generated records identify their inputs and regeneration method. Dated review records are archival evidence: retain their findings and validation context, add a current successor when known, and do not rewrite them to represent later source.
