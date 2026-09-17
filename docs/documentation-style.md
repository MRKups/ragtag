---
title: Documentation style
normative-for: [README.md, docs/]
depends: [development-process.md, glossary.md]
status: guide, not a contract. Accuracy overrides this guide.
---

# Documentation style

These guidelines apply unless a specification needs different wording to state its contract accurately.

## What to document

Document purpose, inputs, outputs, guarantees, limits, failure behavior, and authority boundaries. Do not restate source code in English.

Ask whether a compatible implementation needs the sentence. If not, leave it to source.

| Document this | Leave this to source |
| --- | --- |
| Subsystem purpose | Type, function, and field names |
| Inputs, outputs, and intended outcome | Exact error strings |
| Guarantees and limits | Check order, locking, concurrency, and internal structures |
| Wire vocabulary | Source-line references and caller lists |
| Failure and degradation behavior | |
| Authority boundaries | |

Source code overrides conflicting documentation. Prefer shorter wording when it preserves meaning.

## Voice

These rules apply to README text, CLI help, errors, and `docs/`.

**No grammatical em dashes.** Use punctuation or separate sentences. Structural em dashes are allowed in headings, term definitions, and table labels.

**Write plainly and professionally.** Lead with the point. Avoid blog, sales, chatbot-like, or defensive wording.

**State settled behavior plainly.** Explain genuine ambiguity and its cause.

Avoid: "it is important to note", "simply", "essentially", "in order to", "leverage", "robust", "powerful", "seamless", "comprehensive", "it's worth mentioning".

## Audience

**User-facing:** README text, CLI help, and errors. Translate internal terms.

**Internal technical:** Specifications, architecture notes, and comments. Precise technical language is appropriate; use [the glossary](glossary.md) for repository vocabulary.

XML workflow configuration is internal technical material. Surfaces that expose it to non-authors must translate its schema and state-machine terms.

## Errors

Every error states what happened and the next action.

Tool errors include structured recovery guidance. API errors include a stable programmatic code and a short safe message. Do not expose raw database, workspace, provider, filesystem, or authorization text.

## One home per concept

Give each concept one normative home. Cross-reference it elsewhere to prevent drift.

Keep planned work separate from implemented behavior. Examples do not prove that a command, fixture, or file exists.

Decision records use structural em-dashed headings. Links across this tree target the anchors those headings generate.
