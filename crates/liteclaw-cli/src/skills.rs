//! CLI handlers for skill subcommands: `lc skills`, `lc skill <id>`,
//! `lc skill run <id> [args]`.

use liteclaw_core::{Claw, Ctx, ExitCode};
use liteclaw_skills::{discover, find, SkillClaw};

/// `lc skills` — list all discovered skills.
pub async fn list(ctx: &Ctx) -> anyhow::Result<ExitCode> {
    let skills = discover();
    if skills.is_empty() {
        eprintln!("no skills found (looked in ~/.agents/skills and {}/.liteclaw/skills)", ctx.cwd.display());
        return Ok(ExitCode::Failure);
    }
    if ctx.json {
        let entries: Vec<serde_json::Value> = skills
            .iter()
            .map(skill_json)
            .collect();
        println!("{}", serde_json::json!({ "skills": entries }));
    } else {
        // Human-readable table: source | id | version | name
        eprintln!("S  {:<28} {:<10} NAME", "ID", "VERSION");
        for s in &skills {
            println!(
                "{:<2} {:<28} {:<10} {}",
                s.source.label(),
                truncate(&s.id, 28),
                s.version.as_deref().unwrap_or("-"),
                truncate(&s.name, 40),
            );
        }
        eprintln!("— {} skill(s); S: G=global, P=project", skills.len());
    }
    Ok(ExitCode::Success)
}

/// `lc skill <id>` — show a skill's full SKILL.md body.
pub async fn show(id: &str, ctx: &Ctx) -> anyhow::Result<ExitCode> {
    let skills = discover();
    let Some(skill) = find(&skills, id) else {
        eprintln!("lc skill: no skill with id '{id}'");
        suggest_closest(&skills, id);
        return Ok(ExitCode::Failure);
    };
    if ctx.json {
        println!("{}", skill_json(skill));
    } else {
        println!("--- {} ({}) ---", skill.id, skill.source.label());
        if let Some(v) = &skill.version {
            println!("version: {v}");
        }
        if let Some(s) = &skill.slug {
            println!("slug:    {s}");
        }
        println!("dir:     {}", skill.dir.display());
        if skill.is_scripted() {
            println!("scripts: {}", skill.scripts.len());
        }
        println!();
        print!("{}", skill.body);
        if !skill.body.ends_with('\n') {
            println!();
        }
    }
    Ok(ExitCode::Success)
}

/// `lc skill run <id> [args]` — run a script-based skill.
pub async fn run(id: &str, args: Vec<String>, ctx: &Ctx) -> anyhow::Result<ExitCode> {
    let skills = discover();
    let Some(skill) = find(&skills, id) else {
        eprintln!("lc skill run: no skill with id '{id}'");
        suggest_closest(&skills, id);
        return Ok(ExitCode::Failure);
    };
    if !skill.is_scripted() {
        eprintln!(
            "lc skill run: '{}' has no executable scripts (prompt-only skill)",
            skill.id
        );
        return Ok(ExitCode::Failure);
    }
    // Take ownership of the matching skill (it's Clone).
    let claw = SkillClaw::new(skill.clone());
    let claw_args = liteclaw_core::ClawArgs::new(args);
    claw.run(&claw_args, ctx).await
}

fn skill_json(s: &liteclaw_skills::Skill) -> serde_json::Value {
    serde_json::json!({
        "id": s.id,
        "name": s.name,
        "description": s.description,
        "version": s.version,
        "slug": s.slug,
        "source": s.source.label(),
        "dir": s.dir.display().to_string(),
        "scripted": s.is_scripted(),
        "script_count": s.scripts.len(),
    })
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}

/// Suggest the closest skill id by prefix when an exact match is missing.
fn suggest_closest(skills: &[liteclaw_skills::Skill], query: &str) {
    let q = query.to_lowercase();
    let mut hits: Vec<&str> = skills
        .iter()
        .map(|s| s.id.as_str())
        .filter(|id| id.contains(&q) || q.contains(id))
        .take(5)
        .collect();
    if hits.is_empty() {
        // Fall back to a couple of examples so the user sees valid ids.
        hits = skills.iter().take(5).map(|s| s.id.as_str()).collect();
    }
    if !hits.is_empty() {
        eprintln!("did you mean one of: {}", hits.join(", "));
    }
}
