---
name: manual-rag
description: "Vehicle manual QA - search the onboard car-manual knowledge base. Args = the question text. Use for ANY vehicle feature, operation, warning-light, charging or maintenance question."
version: 0.2.0
---

# Manual RAG

Retrieval-augmented QA over the car manual stored in `/workspace/manual/`.
Single entry script `scripts/rag.py`:

- `rag.py "<question>"` — hybrid search (BM25 + vector), prints JSON hits
  (`rank`, `source` with PDF page range, `section`, `chunk_id`,
  `safety_level`, `text`).
- `rag.py --ingest` — rebuild the index after the manual changes.

## Answering rules

- Answer ONLY from the retrieved passages; quote numbers and warning wording
  exactly. If the passages do not contain the answer, say so explicitly.
- For multi-part questions, run one focused search per sub-question.
- End every answer that used passages with:

  参考来源:
  - <source> | <section>
