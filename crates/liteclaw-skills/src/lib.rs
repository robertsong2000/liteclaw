//! liteclaw-skills: discover, parse, and run claw-ecosystem skills.
//!
//! A "skill" is a directory containing a `SKILL.md` (YAML frontmatter + markdown
//! body), optionally a `scripts/` folder. Skills are discovered from two roots:
//!   - global:  `~/.agents/skills`
//!   - project: `<cwd>/.liteclaw/skills`
//!
//! Project skills override global ones with the same id.

pub mod identity;
pub mod parser;

use std::path::{Path, PathBuf};

use anyhow::Result;
use liteclaw_core::Claw;

pub use parser::{fold_description, parse, Frontmatter, ParsedSkill};

/// Where a skill was discovered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Global,
    Project,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Global => "G",
            Source::Project => "P",
        }
    }
}

/// A fully-resolved skill.
#[derive(Debug, Clone)]
pub struct Skill {
    /// Canonical id (slug > name.lower > dir-stem-without-version).
    pub id: String,
    /// Frontmatter `name` (verbatim).
    pub name: String,
    /// Frontmatter `description`, folded to a single line for display.
    pub description: String,
    pub version: Option<String>,
    pub slug: Option<String>,
    /// Directory the skill lives in.
    pub dir: PathBuf,
    /// Markdown body after the frontmatter.
    pub body: String,
    pub source: Source,
    /// Paths under `scripts/`, if any (for script-based skills).
    pub scripts: Vec<PathBuf>,
}

impl Skill {
    /// True if this skill ships executable scripts (runnable via [`SkillClaw`]).
    pub fn is_scripted(&self) -> bool {
        !self.scripts.is_empty()
    }

    /// The "main" script (first one alphabetically), if scripted.
    pub fn main_script(&self) -> Option<&Path> {
        self.scripts.first().map(PathBuf::as_path)
    }
}

/// Discover all skills from global + project roots, with project overriding
/// global on id collisions.
///
/// Uses the process cwd for the project root and the user's home dir for the
/// global root. Missing directories are silently treated as empty.
pub fn discover() -> Vec<Skill> {
    let global_root = home::agents_skills_dir();
    let project_root = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".liteclaw/skills");
    discover_from(&global_root, &project_root)
}

/// Same as [`discover`] but with explicit roots — easier to test.
pub fn discover_from(global_root: &Path, project_root: &Path) -> Vec<Skill> {
    let global = scan_dir(global_root, Source::Global);
    let project = scan_dir(project_root, Source::Project);
    merge(global, project)
}

/// Scan one root for skill directories.
fn scan_dir(root: &Path, source: Source) -> Vec<Skill> {
    let mut skills: Vec<Skill> = Vec::new();
    let read = match std::fs::read_dir(root) {
        Ok(r) => r,
        Err(_) => return skills, // missing root is fine
    };
    for entry in read.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if let Some(skill) = load_skill(&path, source) {
            skills.push(skill);
        }
    }
    skills.sort_by(|a, b| a.id.cmp(&b.id));
    skills
}

/// Load and parse a single skill directory.
fn load_skill(dir: &Path, source: Source) -> Option<Skill> {
    let skill_md = dir.join("SKILL.md");
    let content = std::fs::read_to_string(&skill_md).ok()?;
    let parsed = parse(&content).ok()?;
    let dir_name = dir.file_name()?.to_str()?;
    let id = identity::skill_id(
        parsed.frontmatter.slug.as_deref(),
        Some(&parsed.frontmatter.name),
        dir_name,
    );
    let scripts = collect_scripts(dir);
    Some(Skill {
        id,
        name: parsed.frontmatter.name,
        description: fold_description(&parsed.frontmatter.description),
        version: parsed.frontmatter.version,
        slug: parsed.frontmatter.slug,
        dir: dir.to_path_buf(),
        body: parsed.body,
        source,
        scripts,
    })
}

/// Collect executable scripts under `<dir>/scripts/`, sorted by name.
fn collect_scripts(dir: &Path) -> Vec<PathBuf> {
    let scripts_dir = dir.join("scripts");
    let mut out = Vec::new();
    let Ok(read) = std::fs::read_dir(&scripts_dir) else {
        return out;
    };
    for entry in read.flatten() {
        let p = entry.path();
        if p.is_file() {
            out.push(p);
        }
    }
    out.sort();
    out
}

/// Read the interpreter from a script's `#!` shebang line, if present.
/// Returns e.g. `bash` for `#!/bin/bash` or `#!/usr/bin/env bash`.
fn read_shebang_interpreter(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let first_line = bytes.split(|&b| b == b'\n').next()?;
    let s = std::str::from_utf8(first_line).ok()?.trim();
    let s = s.strip_prefix("#!")?;
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // `/usr/bin/env <interp>` form → take the last token.
    if s.contains("/env") {
        return s.split_whitespace().last().map(String::from);
    }
    // `/path/to/interp` form → take the basename.
    s.split_whitespace()
        .next()
        .and_then(|t| t.rsplit('/').next().map(String::from))
}

/// Merge global and project lists; project wins on id collisions.
fn merge(mut global: Vec<Skill>, project: Vec<Skill>) -> Vec<Skill> {
    let project_ids: std::collections::HashSet<&str> =
        project.iter().map(|s| s.id.as_str()).collect();
    global.retain(|s| !project_ids.contains(s.id.as_str()));
    global.extend(project);
    global.sort_by(|a, b| a.id.cmp(&b.id));
    global
}

/// Find a skill by id from a pre-discovered list.
pub fn find<'a>(skills: &'a [Skill], id: &str) -> Option<&'a Skill> {
    skills.iter().find(|s| s.id == id)
}

/// A [`Claw`] wrapper that runs a script-based skill. Registered dynamically so
/// `lc skill run <id>` is possible for every scripted skill discovered.
pub struct SkillClaw {
    pub skill: Skill,
}

impl SkillClaw {
    pub fn new(skill: Skill) -> Self {
        Self { skill }
    }
}

#[async_trait::async_trait]
impl Claw for SkillClaw {
    fn name(&self) -> &'static str {
        // Dynamic name — boxed would change the trait; we expose the id via desc.
        // The CLI dispatches skill run separately, so this name is informational.
        "skill-run"
    }

    fn desc(&self) -> &'static str {
        "Run a script-based skill"
    }

    async fn run(
        &self,
        args: &liteclaw_core::ClawArgs,
        ctx: &liteclaw_core::Ctx,
    ) -> Result<liteclaw_core::ExitCode> {
        let Some(script) = self.skill.main_script() else {
            eprintln!(
                "lc skill run {}: no scripts found",
                self.skill.id
            );
            return Ok(liteclaw_core::ExitCode::Failure);
        };

        // Defender pre-check on every argument the skill will receive.
        for a in &args.positionals {
            let report = ctx.guard_text(a);
            if matches!(report.action, liteclaw_core::defender::Action::Block) {
                eprintln!(
                    "⛔ Defender: skill arg blocked — {} (score {})",
                    report.severity.label(),
                    report.score
                );
                return Ok(liteclaw_core::ExitCode::Failure);
            }
        }

        // Execute the script directly if it's executable; otherwise fall back to
        // `sh <script>` so that +x-less scripts (common in the skill pool, e.g.
        // clawdefender.sh checked in without the executable bit) still run.
        let script_path = script.to_path_buf();
        #[cfg(unix)]
        let is_exec = std::fs::metadata(&script_path)
            .map(|m| {
                use std::os::unix::fs::PermissionsExt;
                m.permissions().mode() & 0o111 != 0
            })
            .unwrap_or(false);
        #[cfg(not(unix))]
        let is_exec = false; // non-unix: always go through the shell fallback.

        let mut cmd = if is_exec {
            tokio::process::Command::new(&script_path)
        } else {
            // No +x bit: run via an interpreter. Prefer the shebang if present,
            // otherwise default to bash (skill-pool scripts are bash-heavy and
            // use features like process substitution that plain sh lacks).
            let interpreter = read_shebang_interpreter(&script_path).unwrap_or_else(|| "bash".to_string());
            let mut c = tokio::process::Command::new(interpreter);
            c.arg(&script_path);
            c
        };
        cmd.args(&args.positionals);
        let status = cmd.status().await;
        match status {
            Ok(s) if s.success() => Ok(liteclaw_core::ExitCode::Success),
            Ok(_) => Ok(liteclaw_core::ExitCode::Failure),
            Err(e) => {
                eprintln!("lc skill run {}: {e}", self.skill.id);
                Ok(liteclaw_core::ExitCode::Failure)
            }
        }
    }
}

/// Home-directory helpers kept local to avoid another dependency.
mod home {
    use std::path::PathBuf;
    pub fn agents_skills_dir() -> PathBuf {
        if let Ok(p) = std::env::var("LITECLAW_SKILLS_DIR") {
            return PathBuf::from(p);
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(home).join(".agents/skills")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_skill(root: &Path, dir: &str, name: &str, desc: &str) {
        let d = root.join(dir);
        fs::create_dir_all(&d).unwrap();
        fs::write(
            d.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {desc}\n---\n# {name}\nbody\n"),
        )
        .unwrap();
    }

    fn write_scripted_skill(root: &Path, dir: &str, name: &str) {
        let d = root.join(dir);
        fs::create_dir_all(d.join("scripts")).unwrap();
        fs::write(
            d.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: scripted.\n---\nbody\n"),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let s = d.join("scripts/run.sh");
            fs::write(&s, "#!/bin/sh\necho ran\n").unwrap();
            fs::set_permissions(&s, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn discovers_global_skills() {
        let g = TempDir::new().unwrap();
        write_skill(g.path(), "alpha-1.0.0", "alpha", "a skill");
        write_skill(g.path(), "beta-2.0.0", "beta", "b skill");
        let p = TempDir::new().unwrap();
        let skills = discover_from(g.path(), p.path());
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].id, "alpha");
        assert_eq!(skills[1].id, "beta");
    }

    #[test]
    fn project_overrides_global_on_same_id() {
        let g = TempDir::new().unwrap();
        write_skill(g.path(), "x-1.0.0", "x", "global");
        let p = TempDir::new().unwrap();
        write_skill(p.path(), "x-9.9.9", "x", "project");
        let skills = discover_from(g.path(), p.path());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].source, Source::Project);
        assert_eq!(skills[0].description, "project");
    }

    #[test]
    fn collects_scripts() {
        let g = TempDir::new().unwrap();
        write_scripted_skill(g.path(), "tool-1.0.0", "tool");
        let p = TempDir::new().unwrap();
        let skills = discover_from(g.path(), p.path());
        assert_eq!(skills.len(), 1);
        assert!(skills[0].is_scripted());
        assert!(skills[0].main_script().is_some());
    }

    #[test]
    fn missing_roots_yield_empty() {
        let skills = discover_from(Path::new("/no/such/global"), Path::new("/no/such/project"));
        assert!(skills.is_empty());
    }

    #[test]
    fn find_by_id() {
        let g = TempDir::new().unwrap();
        write_skill(g.path(), "alpha-1.0.0", "alpha", "a");
        let p = TempDir::new().unwrap();
        let skills = discover_from(g.path(), p.path());
        assert!(find(&skills, "alpha").is_some());
        assert!(find(&skills, "missing").is_none());
    }
}
