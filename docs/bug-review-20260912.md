# Bug review — 2026-09-12

Scope: one review pass over the recently committed manual-image flow, model
transport and answer-stream lifecycle. No new product features were added.
Baseline: `3fddca1`; fixes and regression tests are grouped in this commit.

## Findings and fixes

1. An empty or reasoning-only model response still emitted `Done` after the
   retry budget was exhausted. It now emits an error without a success event.
   A local HTTP/SSE fixture reproduces both empty and reasoning-only responses;
   a second case verifies that a retry with an actual answer still succeeds.
   The exhaustion test failed on the baseline and passed after the fix.
2. Both OpenAI-compatible and native Ollama requests converted transport errors
   into strings, discarding their cause chains. `anyhow::Context` now retains
   the original reqwest error so existing terminal logging reveals the cause.
   Tests cover invalid transport schemes in both protocols. This does not add
   SOCKS support or change proxy settings.
3. Browser EOF without a completion/error event silently left an incomplete
   answer or an image-only bubble. The browser now reports this interruption.
   The Playwright regression failed before the fix and passed afterwards.
4. Visible-answer parsing could loop indefinitely when a stray closing think
   tag preceded an opening tag. Closing-tag search now starts after the current
   opening tag, guaranteeing forward progress. The parser regression covers it.

## Verification

- `cargo test --workspace`: 77 tests passed, plus documentation test targets.
- After the final parser fix, all 8 agent library tests and the `lc` build passed.
- Python suites: `tools/test_image_review.py`,
  `tests/test_manual_image_catalog.py`, `tests/test_manual_image_retrieval.py`,
  `tests/test_question_images.py`: 18 tests passed.
- `node tests/manual-images-ui.cjs`: existing gallery regression and new
  incomplete-stream case passed. Chat responses were mocked; this did not
  measure live provider accuracy.
- `git diff --check`: passed.

## Limits

This pass does not certify absence of other bugs, image-label semantic accuracy,
or container behavior. It did not modify the running user-owned server. Restart
that server to load the rebuilt binary. The known deployment proxy issue still
requires an appropriate launch environment; retaining error causes makes such
failures diagnosable instead of hiding them.
