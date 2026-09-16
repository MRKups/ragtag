---
title: Glossary
normative-for: [repository-specific documentation vocabulary]
depends: []
status: current reference. Definitions describe documentation terms, not source implementation.
---

# Glossary

Use these terms consistently in internal documents.

| Term | Meaning |
| --- | --- |
| Application-local data | Data stored in the operating system location assigned to Ragtag, rather than beside the application binary or in a browser store. |
| Citation | Retrieved source metadata presented with an answer. It identifies the source document and page or logical source page. |
| Derived data | Data regenerated from a retained source document, including chunks, embeddings, and search indexes. |
| Document registry | The SQLite record set that owns document identity, lifecycle state, and indexing provenance. |
| Indexed document | A registry document whose derived chunks have been durably stored and are eligible for retrieval. |
| Ingestion | The process that validates a source document, retains it, extracts text, creates chunks, embeds them, and stores derived data. |
| Logical source page | The single page assigned to a non-paginated text or DOCX source for citation purposes. |
| Managed source | The retained, application-local copy of a user-provided document. It is the canonical evidence for its derived data. |
| Preview | A bounded representation of a managed source used by the interface. It is not a general filesystem reader. |
| Retrieval | The local search that selects indexed chunks to ground an answer. |
| Source of truth | The authority that decides a fact. SQLite owns document lifecycle; retained sources own original evidence; source code and migrations override conflicting documentation. |
| Wire contract | The serialized commands, events, values, and error codes exchanged across the Tauri IPC boundary. |
