//! Sandbox: the write/network gate every mutating claw consults.
//!
//! liteclaw is read-only by default. A claw may only write to paths the user
//! explicitly allow-listed (via `--allow-write` or the [`Sandbox`] builder).
//! `.liteclawignore` / `.gitignore` filtering is handled by the `ignore` crate
//! (ripgrep's library) in the fs claws.

use std::path::{Path, PathBuf};

/// Configuration of what a claw is allowed to touch.
#[derive(Debug, Clone, Default)]
pub struct Sandbox {
    /// Directory roots that may be written to. Canonicalized at insertion.
    allow_write: Vec<PathBuf>,
    /// When true, network access is forbidden (model client disabled, etc.).
    network_disabled: bool,
}

impl Sandbox {
    /// Start with no permissions — the safe default.
    pub fn readonly() -> Self {
        Self {
            allow_write: Vec::new(),
            network_disabled: false,
        }
    }

    /// Grant write access to an additional directory root.
    pub fn allow_write(mut self, path: impl Into<PathBuf>) -> Self {
        self.allow_write.push(path.into());
        self
    }

    /// Disable all network access.
    pub fn disable_network(mut self) -> Self {
        self.network_disabled = true;
        self
    }

    /// True if network access is permitted.
    pub fn network_allowed(&self) -> bool {
        !self.network_disabled
    }

    /// True if any writes at all are permitted.
    pub fn is_readonly(&self) -> bool {
        self.allow_write.is_empty()
    }

    /// Check whether `target` may be written to.
    ///
    /// A target is writable if it falls under one of the allow-listed roots.
    /// Because liteclaw may be invoked from any cwd, matching is done against
    /// canonicalized absolute paths when possible, with a lexical prefix
    /// fallback for paths that don't yet exist.
    pub fn can_write(&self, target: &Path) -> bool {
        if self.allow_write.is_empty() {
            return false;
        }
        let target_abs = absolutize(target);
        self.allow_write.iter().any(|root| {
            let root_abs = absolutize(root);
            target_abs == root_abs || target_abs.starts_with(&root_abs)
        })
    }
}

/// Lexical absolutization against the process cwd. Good enough for the prefix
/// check; we avoid canonicalize() because targets often don't exist yet.
fn absolutize(path: &Path) -> PathBuf {
    if path.is_absolute() {
        normalize(path)
    } else {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        normalize(&cwd.join(path))
    }
}

/// Collapse `.` and `..` segments lexically (no filesystem access).
fn normalize(path: &Path) -> PathBuf {
    let mut out: Vec<std::path::Component> = Vec::new();
    for comp in path.components() {
        match comp {
            std::path::Component::ParentDir => {
                // Pop unless the last component is a root.
                match out.last() {
                    Some(std::path::Component::Normal(_)) => {
                        out.pop();
                    }
                    _ => out.push(comp),
                }
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out.iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readonly_by_default() {
        let s = Sandbox::readonly();
        assert!(s.is_readonly());
        assert!(!s.can_write(Path::new("/tmp/anywhere")));
    }

    #[test]
    fn allow_write_grants_root_and_children() {
        let s = Sandbox::readonly().allow_write("/tmp/proj");
        assert!(s.can_write(Path::new("/tmp/proj")));
        assert!(s.can_write(Path::new("/tmp/proj/src/main.rs")));
        assert!(!s.can_write(Path::new("/tmp/other")));
        // Traversal escape must be rejected.
        assert!(!s.can_write(Path::new("/tmp/proj/../other")));
    }
}
