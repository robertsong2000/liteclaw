//! Catalog-backed image references and authenticated serving.
use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    std::env::var("MANUAL_IMAGE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".liteclaw/manual-images")
        })
}
fn catalog() -> Vec<Value> {
    std::fs::read(root().join("catalog.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
fn parse_hits(summary: &str) -> Option<Vec<Value>> {
    let payload = summary.strip_prefix("(exit 0)\n").unwrap_or(summary);
    // The tool runner can append stderr after the JSON document.
    serde_json::Deserializer::from_str(payload)
        .into_iter::<Vec<Value>>()
        .next()
        .and_then(Result::ok)
}
pub fn references(summary: &str) -> Vec<Value> {
    references_from_catalog(summary, &catalog())
}
pub fn has_image_results(summary: &str) -> bool {
    parse_hits(summary).is_some_and(|hits| hits.is_empty() || hits.iter().any(|hit| hit["images"].is_array()))
}
/// Agent tools execute sequentially; pair each result with its actual invocation.
#[derive(Default)]
pub struct ImageEventTracker {
    manual_pending: bool,
}
impl ImageEventTracker {
    pub fn consume(&mut self, event: &Value) -> Option<Vec<Value>> {
        if event["type"] == "tool_start" {
            self.manual_pending = event["tool"] == "skill_run"
                && event["arguments"]["id"] == "manual-rag";
        } else if event["type"] == "tool_result" {
            let manual = std::mem::take(&mut self.manual_pending);
            let summary = event["summary"].as_str().unwrap_or_default();
            if manual && event["tool"] == "skill_run" && event["ok"] == true && has_image_results(summary) {
                return Some(references(summary));
            }
        }
        None
    }
}
fn references_from_catalog(summary: &str, catalog: &[Value]) -> Vec<Value> {
    let hits = parse_hits(summary).unwrap_or_default();
    let mut out = Vec::new();
    let mut refs: Vec<_> = hits
        .iter()
        .flat_map(|hit| hit["images"].as_array().into_iter().flatten())
        .collect();
    // A later, topic-specific page must not be crowded out by the first hit's
    // generic figures. Stable ordering retains retrieval rank for ties.
    refs.sort_by_key(|r| std::cmp::Reverse(r["priority"].as_u64().unwrap_or(0).min(2)));
    for reference in refs {
        if out.len() >= 6 { break; }
        let Some(id) = reference["id"].as_str().filter(|s| valid_id(s)) else {
            continue;
        };
        if let Some(item) = catalog.iter().find(|i| i["id"] == reference["id"]) {
            if let Some(role @ ("primary" | "supporting" | "reference")) = reference["role"].as_str() {
                let limit = match role { "primary" => 1, "supporting" => 2, _ => 3 };
                if out.iter().any(|v: &Value| v["id"] == id)
                    || out.iter().filter(|v| v["role"] == role).count() >= limit {
                    continue;
                }
                out.push(json!({"id":id,"page":item["pdf_page"],
                    "caption":item["alt_text"],"kind":item["kind"],"role":role,
                    "source":item["source_file"],"url":format!("/api/manual-images/{id}")}));
                continue;
            }
            // Show a complete source page when it contains multiple crops;
            // this preserves all callouts while using one gallery slot.
            let page = item["pdf_page"].as_u64();
            let source = item["source_file"].as_str();
            if page.is_some()
                && source.is_some()
                && out.iter().any(|v: &Value| {
                    v["page"] == item["pdf_page"] && v["source"] == item["source_file"]
                })
            {
                continue;
            }
            let multiple = catalog
                .iter()
                .filter(|i| {
                    i["pdf_page"] == item["pdf_page"]
                        && i["source_file"] == item["source_file"]
                        && i["kind"] == "illustration"
                })
                .take(2)
                .count()
                == 2;
            let item = if multiple {
                catalog
                    .iter()
                    .find(|i| {
                        i["pdf_page"] == item["pdf_page"]
                            && i["source_file"] == item["source_file"]
                            && i["kind"] == "source_page"
                    })
                    .unwrap_or(item)
            } else {
                item
            };
            let id = item["id"].as_str().filter(|s| valid_id(s)).unwrap_or(id);
            if out.iter().any(|v: &Value| v["id"] == id) {
                continue;
            }
            out.push(json!({"id":id,"page":item["pdf_page"],
                    "caption":item["alt_text"],"kind":item["kind"],"role":"reference",
                    "source":item["source_file"],"url":format!("/api/manual-images/{id}")}));
            if out.len() == 6 {
                return out;
            }
        }
    }
    out
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn image_path(root: &Path, file: &str) -> Option<PathBuf> {
    let base = root.canonicalize().ok()?;
    let path = base.join(file).canonicalize().ok()?;
    (path.starts_with(&base)
        && path.extension().and_then(|x| x.to_str()) == Some("png")
        && path.is_file())
    .then_some(path)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn only_manual_skill_results_update_images_including_empty_results() {
        let mut events = super::ImageEventTracker::default();
        events.consume(&json!({"type":"tool_start","tool":"bash","arguments":{}}));
        assert!(events.consume(&json!({"type":"tool_result","tool":"bash","ok":true,"summary":"[{\"images\":[]}]"})).is_none());
        events.consume(&json!({"type":"tool_start","tool":"skill_run","arguments":{"id":"other"}}));
        assert!(events.consume(&json!({"type":"tool_result","tool":"skill_run","ok":true,"summary":"[]"})).is_none());
        events.consume(&json!({"type":"tool_start","tool":"skill_run","arguments":{"id":"manual-rag"}}));
        assert_eq!(events.consume(&json!({"type":"tool_result","tool":"skill_run","ok":true,"summary":"(exit 0)\n[]"})), Some(vec![]));
        events.consume(&json!({"type":"tool_start","tool":"skill_run","arguments":{"id":"manual-rag"}}));
        assert!(events.consume(&json!({"type":"tool_result","tool":"skill_run","ok":true,"summary":"not JSON"})).is_none());
    }

    #[test]
    fn selected_crop_is_not_replaced_by_other_figures_on_same_page() {
        let catalog = vec![
            json!({"id":"manual-beam","source_file":"manual.pdf","pdf_page":145,"kind":"illustration"}),
            json!({"id":"auto-beam","source_file":"manual.pdf","pdf_page":145,"kind":"illustration"}),
            json!({"id":"whole-page","source_file":"manual.pdf","pdf_page":145,"kind":"source_page"}),
        ];
        let out = super::references_from_catalog(&json!([{"images":[
            {"id":"manual-beam","role":"primary"}, {"id":"whole-page","role":"reference"}
        ]}]).to_string(), &catalog);
        assert_eq!(out[0]["id"], "manual-beam");
        assert_eq!(out[0]["role"], "primary");
        assert_eq!(out[1]["role"], "reference");
        assert!(super::has_image_results(r#"[{"images":[]}]"#));
        assert!(super::has_image_results("[]"));
        assert!(!super::has_image_results("ordinary tool output"));
    }

    #[test]
    fn files_and_symlinks_cannot_escape_catalog_root() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temporary =
            std::env::temp_dir().join(format!("liteclaw-images-{}-{nonce}", std::process::id()));
        let root = temporary.join("images");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("ok.png"), b"fixture").unwrap();
        std::fs::write(temporary.join("secret.png"), b"outside").unwrap();
        std::fs::write(root.join("config.json"), b"{}").unwrap();
        assert!(super::image_path(&root, "ok.png").is_some());
        assert!(super::image_path(&root, "../secret.png").is_none());
        assert!(super::image_path(&root, "config.json").is_none());
        assert!(super::image_path(&root, "missing.png").is_none());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(temporary.join("secret.png"), root.join("linked.png"))
                .unwrap();
            assert!(super::image_path(&root, "linked.png").is_none());
        }
        std::fs::remove_dir_all(&temporary).unwrap();
    }

    #[test]
    fn catalog_is_authoritative_and_references_are_bounded() {
        let catalog: Vec<_> = (0..10)
            .map(|i| json!({"id":format!("image-{i}"),"pdf_page":i,"alt_text":"trusted"}))
            .collect();
        let refs: Vec<_> = (0..10)
            .map(|i| json!({"id":format!("image-{i}"),"caption":"untrusted"}))
            .collect();
        let summary =
            json!([{"images":[{"id":"unknown"},{"id":"image-0"}]},{"images":refs}]).to_string();
        let out = super::references_from_catalog(&summary, &catalog);
        assert_eq!(out.len(), 6);
        assert_eq!(out[0]["id"], "image-0");
        assert_eq!(out[5]["id"], "image-5");
        assert!(out.iter().all(|i| i["caption"] == "trusted"));
    }

    #[test]
    fn malformed_ids_do_not_create_image_urls() {
        let catalog = vec![json!({"file":"no-id.png"}), json!({"id":"../secret"})];
        let out =
            super::references_from_catalog(r#"[{"images":[{}, {"id":"../secret"}]}]"#, &catalog);
        assert!(out.is_empty());
    }

    #[test]
    fn topic_matched_page_is_not_crowded_out_by_earlier_generic_figures() {
        let catalog: Vec<_> = (0..8).map(|i| json!({"id":format!("image-{i}")})).collect();
        let refs: Vec<_> = (0..8)
            .map(|i| json!({"id":format!("image-{i}"),"priority":if i == 7 { 2 } else { 0 }}))
            .collect();
        let out = super::references_from_catalog(&json!([{"images":refs}]).to_string(), &catalog);
        assert_eq!(out[0]["id"], "image-7");
        assert_eq!(out.len(), 6);
    }

    #[test]
    fn many_crops_from_one_page_do_not_hide_other_source_pages() {
        let mut catalog: Vec<_> = (0..8)
            .map(|i| json!({"id":format!("crop-{i}"),"source_file":"manual.pdf","pdf_page":35}))
            .collect();
        catalog.push(json!({"id":"charging","source_file":"manual.pdf","pdf_page":43}));
        let refs: Vec<_> = catalog.iter().map(|i| json!({"id":i["id"]})).collect();
        let out = super::references_from_catalog(&json!([{"images":refs}]).to_string(), &catalog);
        assert!(out.iter().any(|i| i["id"] == "charging"));
    }

    #[test]
    fn skill_envelope_preserves_json_hits() {
        let hits =
            super::parse_hits("(exit 0)\n[{\"images\":[{\"id\":\"test\"}]}]\n[stderr]\nwarning");
        assert_eq!(hits.unwrap()[0]["images"][0]["id"], "test");
        assert!(super::parse_hits("not json").is_none());
        assert!(super::parse_hits("(exit 0)\n[{truncated").is_none());
    }
}
pub async fn serve(axum::extract::Path(id): axum::extract::Path<String>) -> Response {
    if !valid_id(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let entries = catalog();
    let Some(item) = entries.iter().find(|i| i["id"].as_str() == Some(&id)) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(file) = item["file"].as_str() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(path) = image_path(&root(), file) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::fs::read(path).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "image/png"),
                (header::CACHE_CONTROL, "private, max-age=3600"),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
