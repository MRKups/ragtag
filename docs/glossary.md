---
title: Glossary
normative-for: [repository-specific documentation vocabulary]
depends: []
status: current reference. Definitions describe documentation terms, not source implementation.
---

# Glossary

Use these terms consistently.

| Term | Meaning |
| --- | --- |
| Application-local data | Data in Ragtag's OS-assigned directory, not beside the binary or in browser storage. |
| Citation | Retrieved source metadata shown with an answer, identifying the document and physical or logical source page. |
| Derived data | Regenerated data from a retained source, including chunks, embeddings, and search indexes. |
| Document registry | The SQLite record set that owns document identity, lifecycle state, and indexing provenance. |
| Indexed document | A registry document whose derived chunks have been durably stored and are eligible for retrieval. |
| Ingestion | Validating and retaining a source, extracting text, creating chunks, embedding them, and storing derived data. |
| Logical source page | The single page assigned to a non-paginated text or DOCX source for citation purposes. |
| Managed source | The application-local retained copy of a user document and canonical evidence for its derived data. |
| Preview | A bounded interface representation of a managed source, not a general filesystem reader. |
| Retrieval | The local search that selects indexed chunks to ground an answer. |
| Source of truth | Authority for a fact: SQLite owns document lifecycle, retained sources own original evidence, and source code/migrations override conflicting documentation. |
| Wire contract | The serialized commands, events, values, and error codes exchanged across the Tauri IPC boundary. |
