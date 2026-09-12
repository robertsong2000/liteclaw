# Illustrated manual answers

## Question-specific selection revision (2026-09-12)

Latest user decision: restore runtime DeepSeek selection over candidate textual
descriptions. Keep offline annotations and the pending human-review queue.
Pixel-based multimodal annotation/validation remains deferred. This supersedes
the offline-only runtime selection notes below.

User approved replacing the page-gallery experience with an answer-oriented
selection: one main figure before answer text, up to two genuinely complementary
figures, and collapsed original-page references below. This supersedes the
initial page-diversity rules and six-visible-card acceptance notes below.

Implemented local per-crop source context, R5 symbol-and-label crops, adjacent
page candidate recall, question/context lexical shortlisting, offline annotations,
and a human-review queue. Selected figures are not
replaced with full pages. Roles persist with history; legacy cards are folded.
The current regenerated catalog contains 1,012 assets, including 269 new
symbol/label crops; 1,012 offline annotations and 605 review-queue entries were
generated. Source PDFs, text JSONL and text vectors are unchanged. Online
multimodal selection is intentionally deferred.

Revision verification:

- 33 Rust tests and release build passed, including preserving the selected
  crop when other figures share its page and clearing empty image results.
- Six PDF/selection tests and four retrieval tests passed. Red/green evidence
  covers distinguishing same-page main/automatic-beam context, excluding adjacent
  warning rows, preserving specific symbols in a large shortlist, cross-page
  automatic-beam candidates, and invalid/duplicate model-selected IDs.
  Timeout and old-catalog cases verify that unverified pictures are not promoted.
- Browser regression passed: main image before text, folded/lazy source pages,
  no duplicate gallery on tool transitions, history restore, authentication,
  enlargement, mobile, and no stale references or provider metadata leakage.
  Review-driven regressions also verify one main-image request across streaming
  and tool transitions, one original-page request across repeated expansion,
  and clearing the gallery/persisted metadata after an empty replacement.
- Image events now track the actual sequential `skill_run(id=manual-rag)` call:
  legitimate empty arrays clear images, malformed and unrelated tool results do
  not. The parser distinguishes a valid empty array from a parse failure.
- Six live browser cases passed: manual main beam (figure-1-145), automatic main
  beam (figure-2-145), fuse location/layout, charging connection/disconnection,
  tyre label, and washer warning (symbol-7-136). Explicit skill execution must
  succeed without bash fallback. Auth/path safety and history roundtrip passed.
- Read-only Linux Python container ran the updated retrieval and remote image
  selection successfully with mounted Linux source/index/image paths.
- Visually inspected original page 145, the specific washer-symbol crop, and
  final desktop/mobile answer screenshots. Main figures precede the explanation;
  unrelated page figures are not visible by default.

Independent review of this revision approved after the streaming image-reuse
and manual-only empty-result event fixes. The final six-case live suite passed
again against the rebuilt/restarted native server. All 1,012 PNGs passed
integrity verification. No required review findings remain.

Remaining limitations: selection
uses extracted text rather than image understanding, is model-dependent, and
adds a bounded API call. Missing configuration/failure yields a text-only answer.
The original Docker registry limitation described below has not been changed.

## Objective

Vehicle questions return grounded text together with relevant original manual
illustrations. Preserve source PDF/page metadata and support local and container
deployments. Images must persist with saved conversations and be viewable at full
size. Do not infer safety-critical values from unreviewed illustrations.

## Evidence

- The source dataset has 354 JSONL records and an existing three-entry image
  reference manifest (fuse layout, tyre label, anticorrosion form).
- Current retrieval returns text only. Source page ranges survive indexing.
- Both automatic RAG injection and explicit manual-rag tool calls need image
  attachment support.
- The frontend stores messages in history but currently renders only text and
  user-uploaded images. Streaming rerenders replace the answer DOM.

## Implementation sequence

1. Build a reproducible image catalog from the original PDF and reference
   manifest. Preserve page numbers, descriptions, review state and stable IDs.
   Provide original page previews as context, clearly distinguished from crops.
2. Associate catalog images with JSONL chunks using source/page relationships
   and explicit reference metadata. Preserve associations in index metadata;
   avoid re-embedding unchanged text solely to add image metadata.
3. Return deduplicated relevant image references from retrieval. Bound image
   counts; do not attach unrelated pages or substitute generic illustrations.
4. Serve only catalog-approved image files through authenticated routes. Block
   traversal and arbitrary file access. Keep deployment paths server-side.
5. Emit structured source-image events for automatic RAG and tool-driven RAG.
   Keep display attachments separate from provider message payloads.
6. Render source cards with captions/page numbers, thumbnails and an accessible
   enlarged view. Persist and restore attachments with conversation history;
   support streaming, mobile layouts, loading failures and text-only answers.
7. Document source/index/image configuration for native and Docker operation.

## Acceptance evidence

- Charging, tyre pressure and fuse-location questions produce relevant manual
  images with matching PDF pages; fuse tables link to the physical layout.
- A text-only/no-match question has no fabricated image attachment.
- Both auto-RAG and explicit skill execution expose attachments.
- Reloading a saved conversation retains its source cards.
- Unauthorized and traversal image requests cannot read files.
- Build/tests and a real browser screenshot demonstrate the final behavior,
  including enlarged viewing and a narrow viewport.

## Current status

Implemented PDF page/crop catalog generation, runtime image associations,
authenticated image routes, automatic/tool-driven SSE attachments, history
metadata, gallery/modal rendering, and a mobile layout correction. Generated
743 assets locally (including small warning-symbol pages). No source PDF/JSONL
edits or re-embedding were needed.

Verified on 2026-09-12:

- Verified PNG integrity for every catalog entry: 333 source-page previews and
  410 illustration crops, covering 333 distinct PDF pages (743 valid PNGs).
- `cargo test -p liteclaw-web -p liteclaw-agent -p liteclaw-skills`: 31 tests passed.
- Release build passed; local server restarted with the new binary.
- Latest live browser test passed for automatic fuse/tyre/warning-light RAG and
  explicit charging skill calls, authenticated PNG retrieval (200), unauthorized retrieval (401), unknown
  and traversal IDs (404), server-history roundtrip/cleanup, modal and 390px width.
- Visually inspected the fuse crop, charging crop, complete fuse-location page,
  mobile charging cards and mobile enlarged fuse image.
- Regressions reproduced and fixed: animation-frame overwrite of final gallery;
  skill envelope JSON parsing and 8KB truncation; misleading cross-section
  captions; neighboring wiper image selected for a fuse question.
- Catalog and retrieval Python tests passed, including no image attachments for
  a low-confidence unmatched query. Frontend regression tests passed for reload,
  unavailable-image placeholders, text-only history, and latest-search updates.
- Rust regressions verify catalog-only IDs, bounded/deduplicated references,
  page diversity, topic priority, invalid IDs, and file/symlink path containment.
- Reviewed raster/vector object coverage and added original-page fallback for
  small warning icons and meaningful non-rectangular vector artwork. Inspected
  the actual generated warning-light page 136 and real tyre-answer screenshot.
- Docker Compose merged configuration validates and Docker daemon is available.
  A Linux `python:3.11-slim` container successfully ran the actual RAG script with
  the Compose-equivalent Linux source/index/image paths, read-only host data,
  and remote bge-m3 embeddings.
- Full application container validation also passed: in an isolated cached
  `python:3.11-slim` (Debian 13 ARM64) container, installed Rust 1.88, copied the
  current Rust sources into `/build`, and compiled with `cargo +1.88.0 build
  --release --offline --locked`. Started the resulting Linux `lc` binary with
  host code/data read-only mounted, Linux path overrides, and port 10099. Both
  `manual-images-live.cjs` (all four cases) and `manual-images-ui.cjs` passed
  against that container, including image serving, auth, persistence and mobile.
- Environment limitation: the unmodified Dockerfile's base-image pull through
  this machine's configured Docker Hub mirror failed with 403; direct official
  pulls were also slow/retrying. The alternate full-container validation above
  avoids that host networking issue, but is not a claim that the original
  Dockerfile's image download succeeded. Global Docker settings were not changed.

Independent review completed with APPROVE after two P2 fixes:

- Explicit manifest chunk links are merged with automatic page links. The R5
  corpus image manifest now links the page-351 allocation table to page 350.
  A deterministic test using only the actual indexed table hit failed before
  the fix and passed after rebuilding the catalog. PDF and text chunks remain
  unchanged; no re-embedding was necessary.
- New image-result events replace previous attachments rather than retaining
  stale cards. The browser regression failed with six cards before the fix and
  passed with only the latest card afterwards, natively and in Linux.

Completion audit: source extraction/catalog (743 assets), runtime associations
including cross-page references, catalog-only authenticated serving, automatic
and explicit-tool SSE, provider metadata isolation, gallery/enlargement, history,
mobile/error/text-only behavior, and native/container setup are implemented and
covered by the acceptance evidence above. Image associations join existing
indexed source/page/chunk metadata at retrieval time, rather than duplicating
the catalog into vectors. No required review findings remain.

Non-blocking follow-up: PNG rendering could use atomic temporary-file promotion
to improve recovery from interrupted future asset builds. All current generated
PNGs passed integrity checks. The Docker registry limitation remains as stated.
