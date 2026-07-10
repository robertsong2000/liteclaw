//! SKILL.md parser: splits frontmatter from body, then deserializes the
//! frontmatter with `serde_yaml`.
//!
//! Only `name` and `description` are required; everything else is optional and
//! loosely typed. `description` may be plain, quoted, or a YAML block scalar
//! (`>` folded or `|` literal) — we fold it to a single line for display.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

/// The fields we actually read out of the frontmatter. Unknown fields are
/// ignored (serde default), so the polymorphic `metadata` and niche keys don't
/// break parsing.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Frontmatter {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

/// Parsed SKILL.md: the frontmatter and the markdown body that follows it.
#[derive(Debug, Clone)]
pub struct ParsedSkill {
    pub frontmatter: Frontmatter,
    pub body: String,
}

/// Parse a SKILL.md document. Returns an error if there is no frontmatter block
/// or the frontmatter is not valid YAML.
pub fn parse(content: &str) -> Result<ParsedSkill> {
    let (fm_text, body) =
        split_frontmatter(content).context("SKILL.md must start with a `---` frontmatter block")?;
    let frontmatter: Frontmatter = serde_yaml::from_str(fm_text)
        .with_context(|| "failed to parse SKILL.md frontmatter as YAML")?;
    if frontmatter.name.is_empty() && frontmatter.description.is_empty() {
        return Err(anyhow!(
            "SKILL.md frontmatter has neither `name` nor `description`"
        ));
    }
    Ok(ParsedSkill {
        frontmatter,
        body: body.to_string(),
    })
}

/// Split off the leading `---\n...\n---` block, returning (frontmatter_yaml, body).
fn split_frontmatter(content: &str) -> Option<(&str, &str)> {
    let content = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))?;
    // Find the closing `---` on its own line.
    let close = find_closing_fence(content)?;
    let fm = &content[..close.idx];
    let body = &content[close.body_start..];
    Some((fm, body))
}

struct Fence {
    idx: usize,
    body_start: usize,
}

fn find_closing_fence(after_open: &str) -> Option<Fence> {
    let mut search_from = 0usize;
    loop {
        let rel = after_open[search_from..].find("---")?;
        let abs = search_from + rel;
        // Must be at line start (column 0 or preceded by newline).
        let at_line_start = abs == 0 || after_open.as_bytes().get(abs - 1) == Some(&b'\n');
        if at_line_start {
            // Body starts after the fence line (including its newline).
            let after_fence = &after_open[abs + 3..];
            let body_start = abs
                + 3
                + after_fence
                    .chars()
                    .take_while(|&c| c == '\n')
                    .count()
                    .min(1);
            // Prefer consuming exactly one trailing newline if present.
            let body_start = if after_fence.starts_with('\n') {
                abs + 4
            } else if after_fence.starts_with("\r\n") {
                abs + 5
            } else {
                body_start
            };
            return Some(Fence {
                idx: abs,
                body_start,
            });
        }
        search_from = abs + 3;
    }
}

/// Fold a (possibly multi-line) description into a single display line:
/// collapse runs of whitespace (including newlines) into single spaces.
pub fn fold_description(desc: &str) -> String {
    let mut out = String::with_capacity(desc.len());
    let mut prev_space = false;
    for ch in desc.trim().chars() {
        if ch.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: &str = "---\nname: clawdefender\ndescription: Security scanner for agents.\n---\n# Body\n\nHello.\n";

    #[test]
    fn parses_plain_frontmatter() {
        let p = parse(PLAIN).unwrap();
        assert_eq!(p.frontmatter.name, "clawdefender");
        assert_eq!(p.frontmatter.description, "Security scanner for agents.");
        assert!(p.body.contains("# Body"));
    }

    #[test]
    fn parses_quoted_description() {
        let md = "---\nname: x\ndescription: \"A股分析, 多行描述。\"\n---\nbody\n";
        let p = parse(md).unwrap();
        assert_eq!(p.frontmatter.description, "A股分析, 多行描述。");
    }

    #[test]
    fn parses_folded_block_description() {
        // `>` folded scalar: lines join with spaces.
        let md = "---\nname: aminer\ndescription: >\n  第一行描述。\n  第二行描述。\n---\nbody\n";
        let p = parse(md).unwrap();
        let folded = fold_description(&p.frontmatter.description);
        assert_eq!(folded, "第一行描述。 第二行描述。");
    }

    #[test]
    fn parses_literal_block_description() {
        // `|` literal scalar: preserves newlines, which we then fold for display.
        let md = "---\nname: steve\ndescription: |\n  line one\n  line two\n---\nbody\n";
        let p = parse(md).unwrap();
        let folded = fold_description(&p.frontmatter.description);
        assert_eq!(folded, "line one line two");
    }

    #[test]
    fn ignores_unknown_frontmatter_fields() {
        // metadata in both inline-JSON and nested-map forms must not break us.
        let md = "---\nname: code\nversion: 1.0.4\nslug: code\nmetadata:\n  author: someone\ntags:\n  - a\n  - b\ndescription: A skill.\n---\nbody\n";
        let p = parse(md).unwrap();
        assert_eq!(p.frontmatter.slug.as_deref(), Some("code"));
        assert_eq!(p.frontmatter.version.as_deref(), Some("1.0.4"));
    }

    #[test]
    fn rejects_missing_frontmatter() {
        assert!(parse("# just a doc\nno frontmatter").is_err());
    }

    #[test]
    fn rejects_empty_frontmatter() {
        assert!(parse("---\n---\nbody\n").is_err());
    }

    #[test]
    fn body_starts_after_fence_newline() {
        let p = parse(PLAIN).unwrap();
        assert!(!p.body.starts_with('\n'));
        assert!(p.body.starts_with("# Body"));
    }
}
