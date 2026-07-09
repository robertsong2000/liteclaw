//! Skill identity normalization.
//!
//! A skill's stable id is derived (in priority order) from:
//!   1. the frontmatter `slug` field, if present;
//!   2. the lowercased `name` field, if present;
//!   3. the directory name with a trailing `-<semver>` or `-\d+` stripped.
//!
//! This mirrors the real-world messiness found in the claw skill pool: many
//! skills publish as directory `<slug>-<version>` while `name` holds a human
//! title (e.g. dir `code-1.0.4`, name `Code`, slug `code`).

/// Derive the canonical skill id from the available signals.
pub fn skill_id(slug: Option<&str>, name: Option<&str>, dir_name: &str) -> String {
    if let Some(slug) = slug.map(normalize) {
        if !slug.is_empty() {
            return slug;
        }
    }
    if let Some(name) = name.map(normalize) {
        if !name.is_empty() {
            return name;
        }
    }
    normalize(strip_version_suffix(dir_name))
}

/// Lowercase + collapse internal whitespace; used for name->id normalization.
fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.trim().chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push('-');
                prev_space = true;
            }
        } else {
            for lc in ch.to_lowercase() {
                out.push(lc);
            }
            prev_space = false;
        }
    }
    out
}

/// Strip a trailing `-<semver>` (e.g. `-1.0.4`) or a trailing `-<digits>`
/// (e.g. `-1` for `clawdefender-1`) from a directory stem.
fn strip_version_suffix(dir_name: &str) -> &str {
    if let Some(idx) = rfind_version_dash(dir_name) {
        return &dir_name[..idx];
    }
    dir_name
}

/// Find the byte index of the dash that begins a trailing version token, if any.
fn rfind_version_dash(s: &str) -> Option<usize> {
    // Find the last `-` followed by `<digits>` and optionally `.<digits>...`.
    for (idx, _) in s.rmatch_indices('-') {
        let tail = &s[idx + 1..];
        // `tail` must be all digits, OR digits.digits.digits...
        if tail.is_empty() {
            continue;
        }
        let first = tail.split('.').next().unwrap();
        if !first.is_empty() && first.bytes().all(|b| b.is_ascii_digit()) {
            // Ensure the whole tail is semver-ish (digits and dots only).
            if tail.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
                return Some(idx);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_slug() {
        assert_eq!(skill_id(Some("code"), Some("Code"), "code-1.0.4"), "code");
    }

    #[test]
    fn falls_back_to_name_when_no_slug() {
        // e.g. a-stock-analysis: dir a-stock-analysis-1.0.0, no slug.
        assert_eq!(
            skill_id(None, Some("a-stock-analysis"), "a-stock-analysis-1.0.0"),
            "a-stock-analysis"
        );
    }

    #[test]
    fn falls_back_to_dirname_when_no_slug_no_name() {
        assert_eq!(skill_id(None, None, "clawdefender-1"), "clawdefender");
    }

    #[test]
    fn strips_semver_suffix_from_dirname() {
        assert_eq!(skill_id(None, None, "memory-1.0.2"), "memory");
        assert_eq!(skill_id(None, None, "memory-1.2.7"), "memory");
    }

    #[test]
    fn does_not_strip_non_version_dash() {
        // `a-stock-analysis` has dashes but none is a version; if there were no
        // slug/name, we must NOT strip `-analysis`.
        assert_eq!(skill_id(None, None, "a-stock-analysis"), "a-stock-analysis");
    }

    #[test]
    fn normalizes_name_with_spaces_to_dashes() {
        // e.g. name "Social Media Scheduler" -> "social-media-scheduler"
        assert_eq!(
            skill_id(None, Some("Social Media Scheduler"), "social-media-scheduler-1.0.0"),
            "social-media-scheduler"
        );
    }

    #[test]
    fn handles_human_title_name() {
        // name "SEO (Site Audit ...)" -> lowercased, spaces to dashes
        assert_eq!(
            skill_id(None, Some("Market Research"), "market-research-1.0.0"),
            "market-research"
        );
    }
}
