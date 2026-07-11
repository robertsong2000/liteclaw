//! Backup hook: before write/edit, snapshot the original file so the agent
//! (or user) can undo. Backups live in ~/.liteclaw/backups/.

use crate::hooks::{Hook, HookContext, PreToolVerdict};
use crate::ToolOutcome;
use std::path::PathBuf;

/// A PreToolUse hook that backs up files before write/edit tools modify them.
pub struct BackupHook;

#[async_trait::async_trait]
impl Hook for BackupHook {
    fn name(&self) -> &'static str {
        "backup"
    }

    async fn pre_tool_use(&self, hc: &HookContext<'_>) -> PreToolVerdict {
        // Only intercept write/edit.
        if !matches!(hc.tool_name, "write" | "edit") {
            return PreToolVerdict::Allow { modified_args: None };
        }
        let Some(path_str) = hc.args.get("path").and_then(|v| v.as_str()) else {
            return PreToolVerdict::Allow { modified_args: None };
        };
        let path = hc.ctx.cwd.join(path_str);
        // Backup only if the file already exists (write creating new files has
        // nothing to roll back to, and edit requires an existing match anyway).
        if path.is_file() {
            if let Ok(content) = std::fs::read(&path) {
                let _ = save_backup(&path, &content);
            }
        }
        PreToolVerdict::Allow { modified_args: None }
    }
}

/// Save a backup of the file content. Path: ~/.liteclaw/backups/{base}_{ts}.
fn save_backup(original: &std::path::Path, content: &[u8]) -> std::io::Result<()> {
    let dir = backup_dir();
    std::fs::create_dir_all(&dir)?;
    let name = original
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let backup_path = dir.join(format!("{ts}_{name}"));
    std::fs::write(&backup_path, content)?;
    // Record the mapping (backup → original) in an index file for undo.
    let index = dir.join("index.txt");
    let line = format!("{}\t{}\n", backup_path.display(), original.display());
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&index)?;
    f.write_all(line.as_bytes())?;
    Ok(())
}

fn backup_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".liteclaw/backups")
}

/// Undo the most recent backup: restore the file to its pre-edit state.
/// Returns a human-readable result.
pub fn undo_last() -> ToolOutcome {
    let index = backup_dir().join("index.txt");
    let lines = match std::fs::read_to_string(&index) {
        Ok(s) => s,
        Err(_) => return ToolOutcome::failed("no backups to undo"),
    };
    // Find the last non-empty line.
    let last_line = match lines.lines().filter(|l| !l.trim().is_empty()).last() {
        Some(l) => l.to_string(),
        None => return ToolOutcome::failed("no backups to undo"),
    };
    let parts: Vec<&str> = last_line.split('\t').collect();
    if parts.len() != 2 {
        return ToolOutcome::failed("corrupt backup index");
    }
    let backup_path = parts[0];
    let original_path = parts[1];
    // Restore.
    match std::fs::copy(backup_path, original_path) {
        Ok(_) => {
            // Remove this entry from the index.
            let remaining: String = lines
                .lines()
                .filter(|l| *l != last_line.as_str())
                .map(|l| format!("{l}\n"))
                .collect();
            let _ = std::fs::write(&index, remaining);
            // Optionally remove the backup file.
            let _ = std::fs::remove_file(backup_path);
            ToolOutcome::ok(format!("restored {original_path} from backup"))
        }
        Err(e) => ToolOutcome::failed(format!("restore failed: {e}")),
    }
}
