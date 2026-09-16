---
title: Documentation style
normative-for: [README.md, docs/]
depends: [development-process.md, glossary.md]
status: guide, not a contract. Where it conflicts with being correct, be correct.
---

# Documentation style

How documentation in this repository should read. These are guidelines. A focused specification
that needs to break one of them to state a contract accurately should break it.

## What to document

Documentation is a guide to the system, not the source code restated in English.

Document the features, capabilities and subsystems: what each one is for, what goes in, what comes
out, and what outcome it is meant to produce. Document the guarantees and, just as importantly,
where they stop.

The test for any sentence: **would someone building a compatible harness in another language need
this?** If yes, it is contract. If it only helps someone editing the code, the source already says
it better and will stay correct longer.

| Document this | Leave this to the source |
|---|---|
| What the subsystem is for | Type, function and field names |
| Inputs, outputs, intended outcome | Exact error message strings |
| Guarantees, and where they stop | Order of checks inside a function |
| Wire vocabulary: event types, command kinds, error codes | Locking, concurrency mechanics, internal structures |
| Failure and degradation behaviour | Line references into source files |
| Where one subsystem's authority ends and another begins | Which functions currently have no caller |

Source code is more authoritative than any document here. When the two disagree, the source is
right and the document is a bug.

Prefer the shorter document. Cutting a sentence that restates its neighbour is always correct.

## Voice

These rules apply to everything a person reads: CLI help, README, error messages, and the
specifications under `docs/`.

**No em-dashes in prose.** Mid-sentence they read as machine-written. Use a comma, a colon, a
period, or a new sentence. Replace them in existing copy when you touch it.

Em-dashes are fine where they are structural rather than grammatical: separating a heading from its
subject, a term from its definition, or a label from its description in a table row. Those are
layout, not sentences.

**Corporate-professional, friendly, down to earth.** Write like an experienced consultant
explaining something to the person who has to live with the decision. Not a blog post, not a sales
deck.

**No AI-generated tone.** If it reads like a chatbot wrote it, rewrite it. Short, direct sentences
that lead with the point.

**No defensive language.** State what this system does, not what other systems fail to do. "Direct
reports unavailable rollback" is right. "Unlike tools that pretend to roll back" is not.

**No hedging where the answer is settled.** If the source decides it, say it plainly. If the
behaviour genuinely is ambiguous, say that it is ambiguous, and say why.

Avoid: "it is important to note", "simply", "essentially", "in order to", "leverage", "robust",
"powerful", "seamless", "comprehensive", "it's worth mentioning".

## Audience

Two tiers, with different rules about jargon.

**User-facing.** CLI help text, error messages, and README. Written for someone who has not read
the source. Explain or translate technical concepts rather than assuming them. A person hitting an
error should not need to know what an activation owner is.

**Internal technical.** The specifications under `docs/`, architecture notes, and code comments.
Precise technical language is correct here, and the audience is assumed to know the vocabulary in
[glossary.md](glossary.md). These aim for brevity and correctness above all. The em-dash rule and
the ban on defensive language still apply.

XML workflow configuration sits in the second tier. It is authored by someone who has read the
configuration specification, so schema and state-machine vocabulary is appropriate. Any surface
that presents workflow concepts to a non-author must translate them.

## Errors

Every error tells the reader what happened and what to do next. "Failed to start workspace" is not
enough on its own.

Tool errors returned to a model carry a structured recovery instruction alongside the code and
cause, so the model can correct itself without guessing. Keep that pattern: a new structured tool
error supplies a recovery instruction or it is incomplete.

API errors carry a stable code for programs and a short safe message for people. The code is the
contract and does not change casually. The message never carries raw database, workspace, provider,
filesystem, or authorization text.

## One home per concept

A concept has exactly one normative home. Everywhere else cross-references it. A short summary
"for convenience" is a second home that will drift from the first, so do not add one.

Planned work and implemented behaviour stay separate. A specification example is not evidence that
a command, fixture, or file exists.

The decision records in [decisions.md](decisions.md) use em-dashed headings, and links across this
tree target the anchors those headings generate. Those headings are structural and stay as they
are.
