//! Defender rule tables — a faithful Rust port of clawdefender's detection
//! patterns, with three deliberate bug fixes (documented below).
//!
//! Source of truth: `clawdefender.sh` (clawdefender v1.0.0). Patterns are
//! matched case-insensitively (mirrors `grep -qiE`).
//!
//! # Porting decisions
//! 1. **Allowed-domain anchoring (FIX)**: the bash version used an unanchored
//!    substring `grep -qi "$domain"`, so `evil.com/?x=github.com` bypassed
//!    SSRF checks. Here we anchor with `(^|\.|://)` to prevent the bypass.
//! 2. **Regex escaping (FIX)**: the bash WARNING array left metacharacters
//!    unescaped (e.g. `<|endoftext|>` parsed `|` as alternation). Patterns are
//!    escaped here so they match literally where the intent is literal.
//! 3. **Sensitive-files wiring (FIX)**: `SENSITIVE_FILES` was dead code in the
//!    original (never referenced by any validator). It is wired in here as an
//!    INFO-severity hint so it actually contributes to output.
//!
//! All other patterns are kept verbatim (including intentional overlaps).

/// Severity class for a finding. Numeric scores match clawdefender thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub enum Severity {
    /// Score < 40. Clean — no action.
    Clean = 0,
    /// Score 40-69. Warn but allow.
    Warning = 40,
    /// Score 70-89. Block.
    High = 70,
    /// Score >= 90. Block immediately.
    Critical = 90,
}

impl Severity {
    pub fn score(self) -> u32 {
        self as u32
    }

    /// Classify an arbitrary numeric score into a severity bucket.
    pub fn from_score(score: u32) -> Self {
        if score >= Severity::Critical.score() {
            Severity::Critical
        } else if score >= Severity::High.score() {
            Severity::High
        } else if score >= Severity::Warning.score() {
            Severity::Warning
        } else {
            Severity::Clean
        }
    }

    /// CLI label matching clawdefender output.
    pub fn label(self) -> &'static str {
        match self {
            Severity::Critical => "critical",
            Severity::High => "high",
            Severity::Warning => "warning",
            Severity::Clean => "clean",
        }
    }
}

/// A named detection module. Mirrors clawdefender's per-module grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Module {
    PromptInjection,
    CommandInjection,
    CredentialExfil,
    Ssrf,
    PathTraversal,
    SensitiveFiles,
}

impl Module {
    pub fn label(self) -> &'static str {
        match self {
            Module::PromptInjection => "prompt_injection",
            Module::CommandInjection => "command_injection",
            Module::CredentialExfil => "credential_exfil",
            Module::Ssrf => "ssrf",
            Module::PathTraversal => "path_traversal",
            Module::SensitiveFiles => "sensitive_files",
        }
    }
}

/// One compiled rule: the module it belongs to, its severity, and the pattern.
#[derive(Debug)]
pub struct Rule {
    pub module: Module,
    pub severity: Severity,
    /// Pre-escaped literal-ish pattern. Compiled into a regex by the engine.
    pub pattern: &'static str,
}

// ---- Prompt injection: CRITICAL (score 90) ---------------------------------
// Verbatim from clawdefender PROMPT_INJECTION_CRITICAL[]. Duplicates removed.
const PROMPT_INJECTION_CRITICAL: &[&str] = &[
    r"show.*your.*api.?key",
    r"send.*api.?key",
    r"read.*config.*key",
    r"what.*your.*api.?key",
    r"tell me.*password",
    r"show.*password",
    r"read.*[.]env",
    r"contents of.*[.]env",
    r"show.*secret",
    r"send.*credentials",
    r"what.*credentials",
    r"dump.*credential",
    r"paste.*[.]env",
    r"[.]env.*content",
    r"what.*openclaw",
    r"what.*.openclaw",
    r"what.*.clawdbot",
    r"what.*clawdbot",
    r"show.*clawdbot",
    r"send.*clawdbot",
    r"read.*clawdbot",
    r"read.*.clawdbot",
    r"show.*openclaw",
    r"read.*openclaw",
    r"read.*.openclaw",
    r"list.*credentials",
    r"dump.*credentials",
    r"send.*[.]env",
    r"contents.*[.]env",
    r"show.*[.]env",
    r"what.*in.*config",
    r"ignore previous instructions",
    r"ignore all previous",
    r"ignore your instructions",
    r"disregard above",
    r"disregard.*instructions",
    r"disregard.*prompt",
    r"disregard previous",
    r"forget.*instructions",
    r"forget everything",
    r"forget your instructions",
    r"forget everything above",
    r"override your instructions",
    r"new system prompt",
    r"reset to default",
    r"new instructions",
    r"you are no longer",
    r"disable.*safety",
    r"disable.*filter",
    r"no restrictions",
    r"without.*restrictions",
    r"remove.*restrictions",
    r"without.*guidelines",
    r"no.*ethical",
    r"reveal.*system prompt",
    r"show.*system prompt",
    r"print.*system prompt",
    r"what.*instructions.*given",
    r"what.*your instructions",
    r"print.*conversation",
    r"show.*conversation history",
    r"export.*history",
    r"export all.*data",
    r"export.*user.*data",
];

// ---- Prompt injection: WARNING (score 40) ----------------------------------
// Patterns that are literal metacharacter strings are escaped here (port fix #2).
const PROMPT_INJECTION_WARNING: &[&str] = &[
    r"you are now",
    r"your new role",
    r"pretend to be",
    r"act as if",
    r"roleplay as",
    r"hypothetically",
    r"for educational purposes",
    r"SYSTEM:",
    r"\[INST\]",
    r"<<SYS>>",
    r"jailbreak",
    r"DAN mode",
    r"pretend.*DAN",
    r"you're DAN",
    r"for academic",
    r"in a fictional",
    r"in a hypothetical",
    r"imagine a world",
    r"translate.*then execute",
    r"translate.*then run",
    r"base64.*decode",
    r"rot13",
    r"developer mode",
    r"---END",
    r"END OF SYSTEM",
    r"END OF PROMPT",
    r"<\|endoftext\|>",
    r"###.*SYSTEM",
    r"BEGIN NEW INSTRUCTIONS",
    r"STOP IGNORE",
];

// ---- Command injection: CRITICAL (score 90) --------------------------------
const COMMAND_INJECTION: &[&str] = &[
    r"rm -rf /",
    r"rm -rf \*",
    r"chmod 777",
    r"mkfs\.",
    r"dd if=/dev",
    r":\(\)\{ :\|:& \};:",
    r"nc -e",
    r"ncat -e",
    r"bash -i >& /dev/tcp",
    r"/dev/tcp/",
    r"/dev/udp/",
    r"\| bash",
    r"\| sh",
    r"curl.*\| bash",
    r"wget.*\| sh",
    r"base64 -d \| bash",
    r"base64 --decode \| sh",
    r"eval.*\$\(",
    r"python -c.*exec",
];

// ---- Credential exfiltration: CRITICAL (score 90) --------------------------
const CREDENTIAL_EXFIL: &[&str] = &[
    r"webhook\.site",
    r"requestbin\.com",
    r"requestbin\.net",
    r"pipedream\.net",
    r"hookbin\.com",
    r"beeceptor\.com",
    r"ngrok\.io",
    r"curl.*-d.*[.]env",
    r"curl.*--data.*[.]env",
    r"cat.*[.]env.*curl",
    r"POST.*webhook\.site.*API_KEY",
    r"POST.*webhook\.site.*SECRET",
    r"POST.*webhook\.site.*TOKEN",
];

// ---- SSRF: CRITICAL (score 90) ---------------------------------------------
// Used by scan_url(). Loopback, RFC1918 private ranges, cloud metadata.
const SSRF_PATTERNS: &[&str] = &[
    r"localhost",
    r"127\.0\.0\.1",
    r"0\.0\.0\.0",
    r"10\.\d+\.\d+\.\d+",
    r"172\.(1[6-9]|2[0-9]|3[01])\.\d+\.\d+",
    r"192\.168\.\d+\.\d+",
    r"169\.254\.169\.254",
    r"metadata\.google",
    r"\[::1\]",
];

// ---- Path traversal: HIGH (score 70) ---------------------------------------
const PATH_TRAVERSAL: &[&str] = &[
    r".config/openclaw",
    r".openclaw",
    r"the .openclaw",
    r".openclaw directory",
    r".openclaw folder",
    r"openclaw.json",
    r".config/gog",
    r"cat.*[.]env",
    r"read.*[.]env",
    r"show.*[.]env",
    r"/.env",
    r"config.yaml",
    r"config.json",
    r".ssh/id_",
    r".gnupg",
    r"\.\./\.\./\.\.",
    r"/etc/passwd",
    r"/etc/shadow",
    r"/root/",
    r"~/.ssh/",
    r"~/.aws/",
    r"~/.gnupg/",
    r"%2e%2e%2f",
    r"\.\.%2f",
    r"%2e%2e/",
];

// ---- Sensitive files: WARNING (score 40) -----------------------------------
// Wired in here (port fix #3) — was dead code in the bash original. Lowered
// from the nominal INFO to WARNING so it is actually surfaced.
const SENSITIVE_FILES: &[&str] = &[
    r"[.]env",
    r"id_rsa",
    r"\.pem",
    r"secret",
    r"password",
    r"api.key",
    r"token",
];

// ---- SSRF allowlist (bypass domains) ---------------------------------------
// Anchored in the engine (port fix #1) so suffix/prefix spoofing can't bypass.
pub(crate) const ALLOWED_DOMAINS: &[&str] = &[
    "github.com",
    "api.github.com",
    "api.openai.com",
    "api.anthropic.com",
    "googleapis.com",
    "google.com",
    "npmjs.org",
    "pypi.org",
    "wttr.in",
    "signalwire.com",
    "usetrmnl.com",
];

/// Build the full rule table at startup. Compiled into regexes by the engine.
pub(crate) fn all_rules() -> Vec<Rule> {
    let mut rules = Vec::new();
    // Each pattern array is `&'static [&'static str]`, so the closure must
    // take that exact type to keep the `'static` lifetime (avoiding a borrow
    // that would be shortened to the closure's own lifetime).
    fn push_rules(
        rules: &mut Vec<Rule>,
        patterns: &'static [&'static str],
        module: Module,
        severity: Severity,
    ) {
        for &p in patterns {
            rules.push(Rule {
                module,
                severity,
                pattern: p,
            });
        }
    }
    push_rules(
        &mut rules,
        PROMPT_INJECTION_CRITICAL,
        Module::PromptInjection,
        Severity::Critical,
    );
    push_rules(
        &mut rules,
        PROMPT_INJECTION_WARNING,
        Module::PromptInjection,
        Severity::Warning,
    );
    push_rules(
        &mut rules,
        COMMAND_INJECTION,
        Module::CommandInjection,
        Severity::Critical,
    );
    push_rules(
        &mut rules,
        CREDENTIAL_EXFIL,
        Module::CredentialExfil,
        Severity::Critical,
    );
    push_rules(&mut rules, SSRF_PATTERNS, Module::Ssrf, Severity::Critical);
    push_rules(
        &mut rules,
        PATH_TRAVERSAL,
        Module::PathTraversal,
        Severity::High,
    );
    push_rules(
        &mut rules,
        SENSITIVE_FILES,
        Module::SensitiveFiles,
        Severity::Warning,
    );
    rules
}
