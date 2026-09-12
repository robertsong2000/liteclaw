# Illustrated manual answers

The PDF is the image source. JSONL page ranges and chunk IDs (plus adjacent pages
for continued instructions) retrieve candidates, not a ready-to-display gallery.
Each crop carries its nearby original text and PDF layout metadata. Local ranking
shortlists candidates; DeepSeek selects figures for the question. No synthetic vehicle illustrations
or inferred numeric values are introduced. The text embedding index is unchanged.

The answer starts with one primary figure and, only when needed, up to two
complementary operating figures. Original pages are collapsed at the bottom and
loaded on demand. Manual main-beam operation and automatic-main-beam settings
can select different crops from the same PDF page. A warning-symbol question
can show just that symbol and its label, not an entire page of unrelated lamps.

## Generate assets (once per manual revision)

Use Python 3.9+ with `pdfplumber` and Poppler (`pdftoppm`) installed. These are
asset-build dependencies; serving prebuilt images needs neither pdfplumber nor
a local embedding model. The source directory must contain `data/chunks/*.jsonl`,
`data/manifests/image-reference-manifest.jsonl` and the referenced source PDFs.

```sh
python3 tools/build_manual_images.py --help
python3 tools/build_manual_images.py ../renault_r5_manual "$HOME/.liteclaw/manual-images"
```

The builder renders original PDF pages and crops raster illustration regions at
160 dpi, retaining overlaid callout numbers. Stable IDs include the PDF content
hash and page/figure number. Explicit manifest entries take priority (for example,
the fuse layout is on a different page from its allocation table). Explicitly
listed pages always receive a full-page preview. Manifest `chunk_ids` are merged
with automatic same-page links, not overwritten. Cross-page relationships must
name the explanatory chunk explicitly (the R5 fuse-layout entry links
`r5e-2025-table-fuses-allocation-0001` on page 351 to the picture on page 350).
Keep that manifest with the source corpus when regenerating on another machine.
Small pictograms also receive symbol-and-label crops using this R5 manual's
three-column layout. Non-rectangular vector artwork retains a full-page reference.
Decorative frames and text-only pages are skipped. Column-local context is a
layout heuristic, not computer vision; inspect output when using a different PDF.
Unreviewed captions use the actual PDF page heading, not the JSONL chunk title
(chunks sometimes straddle sections), and do not claim visual interpretation.

Output contains PNG files and an atomically replaced `catalog.json`. Re-running
reuses existing images. Keep the catalog and its PNGs together; old unreferenced
files are not served. Generated assets, indexes and credentials should not be
committed to Git.

The builder also writes `annotations.jsonl` and `review-queue.jsonl`. These are
the first offline visual-association pass: they record the crop/page, nearby text,
confidence and reasons an item needs human review. `same-page-multiple-figures` is
a deliberate review flag, not an error; it means the surrounding text may be
shared by several crops. Review queue entries against the PNG and PDF page to
improve the source associations. Conversation-based pixel review is recorded
separately below; model review does not constitute human approval.

## Conversation visual review

`visual-reviews.jsonl` beside the catalog stores separate model visual assessments.
The local snapshot on 2026-09-12 contains model review records for all 679 crops
and 333 source-page images. This measures record coverage, not verified accuracy:
human approvals remain zero, and 584 layout-rule review-queue entries remain.
Those entries are not 584 confirmed errors. The summary's zero model-flagged
items and the layout queue use different criteria; both need to be inspected.
Each record separates
visible content from page-grounded interpretation and retains PDF/image hashes.
Apply saved reviews with `python3 tools/build_manual_image_review.py CATALOG OUTPUT`;
the asset builder also applies them. Changed images/PDF hashes invalidate the
review. Original context/captions are retained; verified descriptions become
catalog captions/context used by DeepSeek. Keep this file with the image assets
when moving to a container. Human review remains pending.

## Native configuration

`~/.liteclaw/manual-rag.json` configures `manual_dir`, `index_dir`, embedding
`base_url`, `model`, and `api_key`. Environment overrides are `MANUAL_DIR`,
`MANUAL_INDEX_DIR`, `EMBED_BASE_URL`, `EMBED_MODEL`, and `EMBED_API_KEY`.
Images default to `~/.liteclaw/manual-images`; set `MANUAL_IMAGE_DIR` for a
different location in both the web process and its inherited skill environment.
Keep secret configuration mode 600.

Runtime selection calls the configured DeepSeek chat endpoint once when candidate
illustrations exist. It sends the question, image IDs, page numbers, captions and
nearby text, not image pixels. The selector uses the default model endpoint from
`~/.liteclaw/config.json`; `IMAGE_SELECT_MODEL`, `IMAGE_SELECT_BASE_URL` and
`IMAGE_SELECT_API_KEY` can override it. It selects one primary figure and up to
two complementary figures. Invalid IDs are rejected; timeout or invalid output
leaves a text-only answer. Offline annotations and the human-review queue remain
available. Human semantic review and online pixel-based selection remain pending.
The independent `/visual-review` page describes the implementation, limitations,
and proposed next steps; the owner-facing `/help` page remains a question map.

## Docker

Build assets on the host and use the existing `~/.liteclaw` volume for both
the index and image catalog. The optional override maps source JSONL and
overrides Mac-only absolute paths without changing the host configuration:

```sh
docker compose -f docker-compose.yml -f docker-compose.manual.yml up -d --build
```

For a different source location set `MANUAL_SOURCE_ROOT` to its host path.
On another machine copy the generated `manual-rag-index` and `manual-images`
directories to that machine's `.liteclaw` directory and provision model
credentials separately. Remote OpenAI-compatible embedding/chat endpoints work
in either deployment; a local Ollama installation is not required. Host native
and container servers cannot both bind port 9999 simultaneously.

## Display and privacy

Only catalog-approved PNGs are accessible through authenticated
`/api/manual-images/:id` requests. Absolute storage paths and credentials are not
image URLs. Answers show up to three selected illustrations plus up to three
folded original-page references, with PDF page numbers. Click to enlarge and
use Escape or Close to dismiss. Selected crops remain individual even when
several figures share a page. Later image-result events replace the gallery
(including clearing it on an empty result), rather than retaining stale images
from an earlier search. Intermediate tool preambles do not duplicate the gallery.
Page numbers are PDF
page positions, which can differ from printed page numbers.

Source cards, including primary/supporting/reference roles, are stored as metadata alongside assistant messages in browser
history (and supported by the server history API), not as base64 history blobs.
They are removed from subsequent model message payloads. Text-only searches and
missing catalogs remain supported. Images unavailable after moving/deleting the
catalog show a placeholder rather than preventing the answer from rendering.
Image candidate retrieval requires a curated keyword association or a text-vector
similarity of at least 0.55 for that hit. This conservative R5/bge-m3 threshold
can omit pictures for weakly matched questions; it is not a universal guarantee
of relevance. Candidates are shortlisted locally and selected by DeepSeek.
Whole pages are never promoted to main images. Legacy history without selection
roles is shown only under folded references. Text retrieval itself is unchanged.

## Verification

`tests/manual-images-ui.cjs` exercises the real browser renderer with controlled
SSE timing. `tests/manual-images-live.cjs` is opt-in and calls the configured
models with the locally generated R5 corpus. Both use an existing Playwright
Node installation (`NODE_PATH` if not installed locally). Live tests create and
remove one uniquely named server-history fixture only when capacity permits.
