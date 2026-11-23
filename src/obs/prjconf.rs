//! Project configuration (`_config`, prjconf): read, validate, clean.
//!
//! A project's prjconf is the file the scheduler reads to decide what a
//! build installs (`Preinstall`, `Support`), prefers (`Prefer`), replaces
//! (`Substitute`) and ignores (`Ignore`). It is plain text with RPM macro
//! expansion and `%if` conditionals, inherited along the repository path:
//! the effective config for a repository is the concatenation of every
//! config on its path, deepest dependency first, own project last.
//!
//! The parser here models the scheduler's view of that concatenation,
//! pinned against the OBS server sources:
//!
//! * The grammar is `Build::read_config`
//!   (obs-build `Build.pm`, `read_config`, master `0f76cc9`): list
//!   directives accumulate across lines (`!*` clears, `!X` removes X);
//!   `Substitute:` is last-wins per key; `%if`/`%define`/macros are
//!   evaluated by RPM macro expansion (`Build::Rpm::parse`, same
//!   commit: `%if` / `%ifarch` / `%ifnarch` / `%ifos` / `%ifnos` and
//!   `%elif` / `%elifarch` / `%elifos`); `Macros:` sections are
//!   verbatim blobs; an unknown keyword warns; a `FromScratch:` that
//!   is not the first content line is a parse error. A trailing
//!   `#...` comment on a directive line is skipped by the checks:
//!   `read_config` tokenizes it (the solver never sees a package
//!   there), but a comment word is not a target, so it is neither
//!   resolution-checked nor counted as a duplicate, and a cleanup
//!   preserves it verbatim. The standard
//!   macros (`$std_macros`: `ix86`, `x86_64`, `arm`, `mips`, `sparc`,
//!   `alpha`, `power64`, `riscv`, `loongarch64`, `nil`) plus
//!   `_target_cpu` (the repository architecture) and `_target_os`
//!   (`linux`) are seeded before parsing, as `read_config` does.
//! * Inheritance is `BSSched::ProjPacks::getconfig` (OBS server
//!   `ProjPacks.pm`, master `e963bb5`): the effective config is the
//!   concatenation along the repo path in reverse, with `%define
//!   _project` / `_repository` / `_is_this_project` / `_is_in_project`
//!   headers, a `FromScratch:` reset, and `Macros:` sections stripped
//!   from the scheduler's view.
//! * The buildconfig endpoint (`GET /build/<project>/<repository>
//!   /_buildconfig`) returns the server-concatenated effective config
//!   with `### from <project>` provenance markers; it still contains
//!   `%if`/macros, so this module evaluates them itself.
//! * The raw config endpoints are `GET /source/<project>/_config`
//!   (an absent config returns an empty string) and `PUT
//!   /source/<project>/_config` (a raw file body; an empty upload
//!   deletes the config; scmsync-controlled projects refuse it).
//!
//! A `Prefer` split across lines takes effect: list directives
//! accumulate, so a multi-line `Prefer` is one list, not a dead form.
//! What the parser actually makes dead is flagged instead: a `!X`
//! removal of a token that was never added, a `!*` clear of an empty
//! list, an overridden `Substitute` line, a duplicate token, and a
//! `%if` branch that is false in a repository and architecture
//! evaluation (cleanup removes it only when it is false in every
//! evaluation).
//!
//! Cleanup removes only lines proven dead in every repository and
//! architecture evaluation — and, by default, only proofs that do
//! not depend on the repository path or architecture at all: a
//! prjconf is inherited, so a line dead in this project's
//! evaluations can be live in a downstream project that inherits
//! this config (different repository paths, architectures and
//! `_repository` / `_project` values). The default cleanup removes
//! same-line duplicate tokens and unconditionally overridden
//! `Substitute` lines; `--include-path-dependent` also removes
//! lines proven dead in every evaluation of this project. Anything
//! that cannot be evaluated (a condition or macro reference the
//! evaluator cannot resolve) poisons only the lists it could touch;
//! those lines stay. A parse error refuses the cleanup outright.
//!
//! Evaluation is bounded: macro expansion has a depth cap, a
//! length cap, a per-evaluation step budget (an empty expansion
//! fans out without growing anything, so the length cap alone does
//! not bound it) and a per-evaluation expanded-byte budget (a
//! short line fanning one large value out thousands of times
//! stores a fresh copy per reference, so the length cap alone
//! does not bound that either), and `%if` expressions have a
//! nesting and token cap — past any cap the input is
//! unevaluatable, never guessed.
//! A trailing `\` joins a line with the next before parsing, the
//! way obs-build joins `%define` values and multi-line conditions.

use super::build_url;
use super::parse_status_ok;
use super::valid_obs_name;
use super::DryRun;
use super::ObsClient;
use super::Owner;
use crate::error::ErrorKind;
use crate::error::GeectlError;
use serde::Serialize;
use std::collections::HashMap;
use std::collections::HashSet;

/// A list directive keyword (lowercase, without the colon). The solver
/// lists are the ones `Build::read_config` accumulates across lines.
const LIST_DIRECTIVES: &[&str] = &[
    "preinstall",
    "vminstall",
    "required",
    "support",
    "keep",
    "prefer",
    "ignore",
    "conflict",
    "runscripts",
    "expandflags",
    "buildflags",
    "publishflags",
    "repourl",
    "registryurl",
    "assetsurl",
    "onlynative",
    "alsonative",
    "ignorerebuild",
];

/// The solver package lists, whose tokens are package names and which
/// the resolution check runs over.
const PACKAGE_LISTS: &[&str] = &[
    "preinstall",
    "vminstall",
    "required",
    "support",
    "prefer",
    "ignore",
    "conflict",
];

/// Every other keyword `read_config` recognises (single-value or
/// section directives). A line starting with any of these is a
/// directive; anything else is an unknown keyword.
const OTHER_DIRECTIVES: &[&str] = &[
    "macros",
    "distmacro",
    "substitute",
    "fileprovides",
    "exportfilter",
    "publishfilter",
    "optflags",
    "order",
    "repotype",
    "type",
    "buildengine",
    "binarytype",
    "patterntype",
    "release",
    "cicntstart",
    "releaseprg",
    "releasesuffix",
    "changetarget",
    "target",
    "hostarch",
    "constraint",
    "singleexport",
    "fromscratch",
];

/// A package-name-shaped token: an optional leading `-` (an avoid in
/// `Prefer:`/`Ignore:`) followed by a name. Anything else is not a
/// package name and the resolution check skips it.
fn looks_like_package(token: &str) -> bool {
    let name = token.strip_prefix('-').unwrap_or(token);
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+.:~-_".contains(c))
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
}

/// The kind of a prjconf finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrjconfFindingKind {
    /// A `Prefer` / `Substitute` / `Support` target that resolves to
    /// no package in any repository on the path and no binary built by
    /// one.
    UnresolvableTarget,
    /// A `!X` removal of a token that was never added to the list.
    DeadRemoval,
    /// A `!*` clear of a list that is already empty.
    DeadClear,
    /// A `Substitute` line for a key a later line sets again; the
    /// earlier line contributes nothing to the final map.
    OverriddenSubstitute,
    /// A token already present in the list when the line runs.
    DuplicateToken,
    /// A line whose keyword the parser does not recognise.
    UnknownKeyword,
    /// A `Prefer: -X` avoid while `Support` / `Preinstall` /
    /// `Required` still carries X.
    ShadowedAvoid,
    /// A `%if` branch false in a (repository, architecture) evaluation.
    DeadBranch,
    /// An `%if` with no matching `%endif`: the frame runs to the
    /// end of the text, so the lines it swallows define nothing.
    UnclosedIf,
    /// A `FromScratch:` that is not the first content line, or a
    /// stray `%endif` / `%elif` / `%else`.
    ParseError,
}

/// The severity of a prjconf finding. Errors refuse a cleanup;
/// warnings are reported and left for a human decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrjconfSeverity {
    Error,
    Warning,
}

impl PrjconfFindingKind {
    /// The snake_case spelling of the kind, as it appears in JSON
    /// and in text renderings.
    pub fn as_str(self) -> &'static str {
        match self {
            PrjconfFindingKind::UnresolvableTarget => "unresolvable_target",
            PrjconfFindingKind::DeadRemoval => "dead_removal",
            PrjconfFindingKind::DeadClear => "dead_clear",
            PrjconfFindingKind::OverriddenSubstitute => "overridden_substitute",
            PrjconfFindingKind::DuplicateToken => "duplicate_token",
            PrjconfFindingKind::UnknownKeyword => "unknown_keyword",
            PrjconfFindingKind::ShadowedAvoid => "shadowed_avoid",
            PrjconfFindingKind::DeadBranch => "dead_branch",
            PrjconfFindingKind::UnclosedIf => "unclosed_if",
            PrjconfFindingKind::ParseError => "parse_error",
        }
    }
}

/// One prjconf finding. `line` is the line in the project's own
/// `_config` when the line comes from that config; lines inherited
/// from another project carry no line number, only the project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrjconfFinding {
    pub kind: PrjconfFindingKind,
    pub severity: PrjconfSeverity,
    /// The repository the finding was evaluated for, when it is
    /// repository-scoped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// The project the line came from.
    pub project: String,
    /// The line in the project's own `_config`, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    pub message: String,
}

/// Something the validator could not evaluate, so it stayed out of the
/// proof: a condition or macro reference the evaluator cannot resolve,
/// a line whose mapping back to the own config failed, or a line
/// inside a dead segment, skipped from evaluation.
/// Skipped items narrow a cleanup; they do not block it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrjconfSkipped {
    /// The repository the item was evaluated for, when scoped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// The project the line came from.
    pub project: String,
    /// The line in the project's own `_config`, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    pub reason: String,
}

/// The result of validating one project's prjconf across its
/// repositories.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrjconfValidation {
    pub project: String,
    pub findings: Vec<PrjconfFinding>,
    pub skipped: Vec<PrjconfSkipped>,
}

/// The result of cleaning one project's prjconf. `cleaned` is the
/// full text that was written (`applied`) or would be written
/// (dry run).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrjconfCleanResult {
    pub project: String,
    pub applied: bool,
    /// The own-config lines removed, 1-based.
    pub removed_lines: Vec<usize>,
    pub cleaned: String,
    pub findings: Vec<PrjconfFinding>,
    pub skipped: Vec<PrjconfSkipped>,
}

/// A macro table: defined names with their raw values, plus names
/// whose value cannot be known (defined inside a branch the evaluator
/// could not resolve, or parameterised). Any use of an unknown name
/// makes the expansion unevaluatable.
#[derive(Debug, Clone, Default)]
struct MacroTable {
    defs: HashMap<String, String>,
    unknown: HashSet<String>,
    /// Macro references expanded so far in this evaluation. A table
    /// is created per evaluation, so the budget is per evaluation:
    /// it bounds the total expansion work even when every expansion
    /// is empty and the length cap never trips.
    steps: std::cell::Cell<u64>,
    /// Bytes of expanded output produced so far in this evaluation.
    /// The length cap bounds one `expand_macros` result, but a line
    /// fanning one large value out thousands of times stores a fresh
    /// copy per reference, so the per-call cap never trips while
    /// memory grows without bound; this budget bounds the total.
    expanded_bytes: std::cell::Cell<usize>,
}

impl MacroTable {
    /// Charge one step of expansion work; the budget is exhausted
    /// past `MAX_EVAL_STEPS`, and the expansion is unevaluatable.
    fn tick(&self) -> Result<(), Unevaluatable> {
        let steps = self.steps.get() + 1;
        self.steps.set(steps);
        if steps > MAX_EVAL_STEPS {
            return Err(Unevaluatable);
        }
        Ok(())
    }

    /// Charge `bytes` of expanded output to this evaluation; past
    /// `MAX_EVAL_EXPANDED_BYTES` the expansion is unevaluatable.
    fn charge(&self, bytes: usize) -> Result<(), Unevaluatable> {
        let total = self.expanded_bytes.get().saturating_add(bytes);
        self.expanded_bytes.set(total);
        if total > MAX_EVAL_EXPANDED_BYTES {
            return Err(Unevaluatable);
        }
        Ok(())
    }
}

/// The macro expansion failed because a referenced name is undefined,
/// unknown, or uses a form the evaluator does not model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Unevaluatable;

// The largest expansion `expand_macros` returns: macro values
// multiply at every level, so the depth cap alone does not bound
// memory. Anything larger is unevaluatable, never guessed.
const MAX_EXPANDED_LEN: usize = 1 << 20;

// The most macro references one evaluation expands: the length cap
// does not bound work when macros expand to `""` (an empty
// expansion fans out without growing anything), so the call budget
// does. A real config expands orders of magnitude fewer references;
// a fan-out like `%a16` over twenty empty `%a0` references each
// trips it. Past the budget the expansion is unevaluatable.
const MAX_EVAL_STEPS: u64 = 1_000_000;

// The most bytes of expanded output one evaluation produces in
// total: the length cap bounds one `expand_macros` result, but a
// line fanning one large value out thousands of times stores a
// fresh copy per reference, so the per-call cap never trips while
// memory grows without bound (a 1 MiB value referenced 2000 times
// on one 6 KB line stores ~4 GB). A real config expands orders of
// magnitude less; past the budget the expansion is unevaluatable.
const MAX_EVAL_EXPANDED_BYTES: usize = 8 << 20;

// The deepest `%if` expression nesting (`(` or `!`): each level
// recurses, so an unbounded expression aborts the process on stack
// overflow instead of returning a mapped error. Past the depth the
// condition is unevaluatable.
const MAX_EXPR_DEPTH: usize = 64;

// The most tokens one `%if` expression holds: bounds the parser's
// work linearly in the expanded text.
const MAX_EXPR_TOKENS: usize = 4096;

// The most repositories `validate_project_config_full` evaluates:
// one buildconfig fetch per repository, so an unbounded project
// spins unboundedly. The largest real project,
// SUSE:SLE-15-SP1:Update, has 26 repositories; 32 leaves 6
// headroom. Past the bound the call refuses outright: a partial
// evaluation would hand `clean` a plan proven dead only in the
// evaluated repositories.
const MAX_PRJCONF_REPOSITORIES: usize = 32;

// The most `/search/owner` lookups one validation issues: one
// sequential request per distinct token otherwise, including
// inherited ones. Past the cap the remaining names are left
// unresolved (never guessed unresolvable) and the cap is reported
// as a skip.
const MAX_PRJCONF_OWNER_LOOKUPS: usize = 100;

/// The remaining owner-lookup budget for one validation, shared
/// across its evaluations.
struct OwnerLookupBudget {
    remaining: usize,
    reported: bool,
}

impl OwnerLookupBudget {
    fn new() -> Self {
        OwnerLookupBudget {
            remaining: MAX_PRJCONF_OWNER_LOOKUPS,
            reported: false,
        }
    }
}

/// Expand RPM macros in `text`. Supports `%%`, `%name`, `%{name}`,
/// `%{?name}`, `%{?name:expr}`, `%{!?name:expr}` and
/// `%{name:-default}`. Anything else with a `%` is unevaluatable.
fn expand_macros(text: &str, macros: &MacroTable, depth: usize) -> Result<String, Unevaluatable> {
    if depth > 16 {
        return Err(Unevaluatable);
    }
    if text.len() > MAX_EXPANDED_LEN {
        return Err(Unevaluatable);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pct) = rest.find('%') {
        if out.len() > MAX_EXPANDED_LEN {
            return Err(Unevaluatable);
        }
        macros.charge(pct)?;
        out.push_str(&rest[..pct]);
        rest = &rest[pct..];
        if let Some(tail) = rest.strip_prefix("%%") {
            macros.charge(1)?;
            out.push('%');
            rest = tail;
            continue;
        }
        if let Some(tail) = rest.strip_prefix("%{") {
            let Some(close) = find_brace_close(tail) else {
                return Err(Unevaluatable);
            };
            let inner = &tail[..close];
            let after = &tail[close + 1..];
            macros.tick()?;
            let expanded = expand_braced(inner, macros, depth)?;
            macros.charge(expanded.len())?;
            out.push_str(&expanded);
            rest = after;
            continue;
        }
        let name: String = rest[1..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() {
            return Err(Unevaluatable);
        }
        if macros.unknown.contains(&name) || !macros.defs.contains_key(&name) {
            return Err(Unevaluatable);
        }
        macros.tick()?;
        let expanded = expand_macros(&macros.defs[&name], macros, depth + 1)?;
        macros.charge(expanded.len())?;
        out.push_str(&expanded);
        rest = &rest[1 + name.len()..];
    }
    macros.charge(rest.len())?;
    out.push_str(rest);
    if out.len() > MAX_EXPANDED_LEN {
        return Err(Unevaluatable);
    }
    Ok(out)
}

/// The index of the `}` closing the `%{` whose body starts at `tail`,
/// counting nested braces.
fn find_brace_close(tail: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in tail.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

/// Expand one `%{...}` body.
fn expand_braced(inner: &str, macros: &MacroTable, depth: usize) -> Result<String, Unevaluatable> {
    if let Some(body) = inner.strip_prefix('?') {
        let (name, expr) = split_colon_expr(body);
        if macros.unknown.contains(&name) {
            return Err(Unevaluatable);
        }
        if macros.defs.contains_key(&name) {
            return match expr {
                Some(expr) => expand_macros(expr, macros, depth + 1),
                None => expand_macros(&macros.defs[&name], macros, depth + 1),
            };
        }
        return Ok(String::new());
    }
    if let Some(body) = inner.strip_prefix("!?") {
        let (name, expr) = split_colon_expr(body);
        if macros.unknown.contains(&name) {
            return Err(Unevaluatable);
        }
        if !macros.defs.contains_key(&name) {
            return match expr {
                Some(expr) => expand_macros(expr, macros, depth + 1),
                None => Err(Unevaluatable),
            };
        }
        return Ok(String::new());
    }
    let name_len = inner
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .map(char::len_utf8)
        .sum::<usize>();
    let name = &inner[..name_len];
    let rest = &inner[name_len..];
    if let Some(default) = rest.strip_prefix(":-") {
        if macros.unknown.contains(name) {
            return Err(Unevaluatable);
        }
        if let Some(value) = macros.defs.get(name) {
            if !value.is_empty() {
                return expand_macros(value, macros, depth + 1);
            }
        }
        return expand_macros(default, macros, depth + 1);
    }
    if !rest.is_empty() {
        return Err(Unevaluatable);
    }
    if macros.unknown.contains(name) || !macros.defs.contains_key(name) {
        return Err(Unevaluatable);
    }
    expand_macros(&macros.defs[name], macros, depth + 1)
}

/// Split a `%{?name...}` / `%{!?name...}` body into the name and the
/// optional `:expr` after it.
fn split_colon_expr(body: &str) -> (String, Option<&str>) {
    let name: String = body
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    let rest = &body[name.len()..];
    match rest.strip_prefix(':') {
        Some(expr) => (name, Some(expr)),
        None => (name, None),
    }
}

/// A value in a `%if` expression.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ExprValue {
    Int(i64),
    Str(String),
}

impl ExprValue {
    /// `Build::Rpm::expr_boolify`: a number is true when non-zero;
    /// a string is true when non-empty — including `"0"`, which the
    /// server treats as true. Only an unquoted `0` (an `Int`) is
    /// false.
    fn truthy(&self) -> bool {
        match self {
            ExprValue::Int(n) => *n != 0,
            ExprValue::Str(s) => !s.is_empty(),
        }
    }
}

/// Evaluate a `%if` condition after macro expansion. Numbers compare
/// numerically; strings compare by equality only (a relational
/// comparison of non-numeric strings is unevaluatable).
fn eval_condition(expr: &str, macros: &MacroTable) -> Result<bool, Unevaluatable> {
    let expanded = expand_macros(expr, macros, 0)?;
    let tokens = tokenize_expr(&expanded)?;
    let mut parser = ExprParser {
        tokens: &tokens,
        pos: 0,
    };
    let value = parser.parse_or(0)?;
    if parser.pos != tokens.len() {
        return Err(Unevaluatable);
    }
    Ok(value.truthy())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExprToken {
    Int(i64),
    Str(String),
    Op(&'static str),
    LParen,
    RParen,
}

/// Tokenize an expanded `%if` expression. A `%` left over after
/// expansion is unevaluatable. So is non-canonical input the server's
/// `expr` rejects: a `-` before a non-digit, and a bare word starting
/// with a digit that is not a plain integer (`15.6`). Anything the
/// tokenizer cannot model is unevaluatable, never guessed.
fn tokenize_expr(expr: &str) -> Result<Vec<ExprToken>, Unevaluatable> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = expr.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if tokens.len() > MAX_EXPR_TOKENS {
            return Err(Unevaluatable);
        }
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '(' {
            tokens.push(ExprToken::LParen);
            i += 1;
            continue;
        }
        if c == ')' {
            tokens.push(ExprToken::RParen);
            i += 1;
            continue;
        }
        if c == '\'' || c == '"' {
            let quote = c;
            let mut s = String::new();
            i += 1;
            while i < chars.len() && chars[i] != quote {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                    s.push(chars[i]);
                } else {
                    s.push(chars[i]);
                }
                i += 1;
            }
            if i >= chars.len() {
                return Err(Unevaluatable);
            }
            i += 1;
            tokens.push(ExprToken::Str(s));
            continue;
        }
        if c == '%' {
            return Err(Unevaluatable);
        }
        if c == '&' || c == '|' {
            let op = if c == '&' { "&&" } else { "||" };
            if i + 1 >= chars.len() || chars[i + 1] != c {
                return Err(Unevaluatable);
            }
            tokens.push(ExprToken::Op(op));
            i += 2;
            continue;
        }
        if c == '=' || c == '!' || c == '<' || c == '>' {
            if i + 1 < chars.len() && chars[i + 1] == '=' {
                let op = match c {
                    '=' => "==",
                    '!' => "!=",
                    '<' => "<=",
                    '>' => ">=",
                    _ => unreachable!(),
                };
                tokens.push(ExprToken::Op(op));
                i += 2;
            } else if c == '<' || c == '>' {
                tokens.push(ExprToken::Op(if c == '<' { "<" } else { ">" }));
                i += 1;
            } else if c == '!' {
                tokens.push(ExprToken::Op("!"));
                i += 1;
            } else {
                return Err(Unevaluatable);
            }
            continue;
        }
        if c == '-' && (i + 1 >= chars.len() || !chars[i + 1].is_ascii_digit()) {
            // A `-` before a non-digit is neither an operator here
            // nor a number: the server's `expr` rejects it.
            return Err(Unevaluatable);
        }
        if c.is_ascii_digit() || c == '-' {
            let start = i;
            if chars[i] == '-' {
                i += 1;
            }
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let num: String = chars[start..i].iter().collect();
            // A number followed immediately by a word character is
            // not a number (e.g. `15.6`): the server's `expr`
            // rejects the bare word, so it is unevaluatable.
            if i < chars.len()
                && (chars[i] == '.' || chars[i].is_ascii_alphanumeric() || chars[i] == '_')
            {
                return Err(Unevaluatable);
            }
            let n: i64 = num.parse().map_err(|_| Unevaluatable)?;
            tokens.push(ExprToken::Int(n));
            continue;
        }
        let mut word = String::new();
        while i < chars.len()
            && !chars[i].is_whitespace()
            && !matches!(
                chars[i],
                '(' | ')' | '&' | '|' | '=' | '!' | '<' | '>' | '\'' | '"'
            )
        {
            word.push(chars[i]);
            i += 1;
        }
        if word.is_empty() {
            return Err(Unevaluatable);
        }
        match word.parse::<i64>() {
            Ok(n) => tokens.push(ExprToken::Int(n)),
            Err(_) => tokens.push(ExprToken::Str(word)),
        }
    }
    Ok(tokens)
}

struct ExprParser<'a> {
    tokens: &'a [ExprToken],
    pos: usize,
}

impl ExprParser<'_> {
    fn peek(&self) -> Option<&ExprToken> {
        self.tokens.get(self.pos)
    }

    fn eat_op(&mut self, op: &'static str) -> bool {
        if self.peek() == Some(&ExprToken::Op(op)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn parse_or(&mut self, depth: usize) -> Result<ExprValue, Unevaluatable> {
        let mut left = self.parse_and(depth)?;
        while self.eat_op("||") {
            let right = self.parse_and(depth)?;
            left = ExprValue::Int(i64::from(left.truthy() || right.truthy()));
        }
        Ok(left)
    }

    fn parse_and(&mut self, depth: usize) -> Result<ExprValue, Unevaluatable> {
        let mut left = self.parse_equality(depth)?;
        while self.eat_op("&&") {
            let right = self.parse_equality(depth)?;
            left = ExprValue::Int(i64::from(left.truthy() && right.truthy()));
        }
        Ok(left)
    }

    fn parse_equality(&mut self, depth: usize) -> Result<ExprValue, Unevaluatable> {
        let mut left = self.parse_relational(depth)?;
        loop {
            if self.eat_op("==") {
                let right = self.parse_relational(depth)?;
                left = ExprValue::Int(i64::from(values_equal(&left, &right)));
            } else if self.eat_op("!=") {
                let right = self.parse_relational(depth)?;
                left = ExprValue::Int(i64::from(!values_equal(&left, &right)));
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_relational(&mut self, depth: usize) -> Result<ExprValue, Unevaluatable> {
        let mut left = self.parse_unary(depth)?;
        loop {
            let op = if self.eat_op("<=") {
                "<="
            } else if self.eat_op(">=") {
                ">="
            } else if self.eat_op("<") {
                "<"
            } else if self.eat_op(">") {
                ">"
            } else {
                break;
            };
            let right = self.parse_unary(depth)?;
            let (ExprValue::Int(a), ExprValue::Int(b)) = (&left, &right) else {
                return Err(Unevaluatable);
            };
            let result = match op {
                "<=" => a <= b,
                ">=" => a >= b,
                "<" => a < b,
                ">" => a > b,
                _ => unreachable!(),
            };
            left = ExprValue::Int(i64::from(result));
        }
        Ok(left)
    }

    fn parse_unary(&mut self, depth: usize) -> Result<ExprValue, Unevaluatable> {
        if self.eat_op("!") {
            if depth >= MAX_EXPR_DEPTH {
                return Err(Unevaluatable);
            }
            let value = self.parse_unary(depth + 1)?;
            return Ok(ExprValue::Int(i64::from(!value.truthy())));
        }
        self.parse_primary(depth)
    }

    fn parse_primary(&mut self, depth: usize) -> Result<ExprValue, Unevaluatable> {
        match self.peek() {
            Some(ExprToken::LParen) => {
                if depth >= MAX_EXPR_DEPTH {
                    return Err(Unevaluatable);
                }
                self.pos += 1;
                let value = self.parse_or(depth + 1)?;
                if self.peek() != Some(&ExprToken::RParen) {
                    return Err(Unevaluatable);
                }
                self.pos += 1;
                Ok(value)
            }
            Some(ExprToken::Int(n)) => {
                let n = *n;
                self.pos += 1;
                Ok(ExprValue::Int(n))
            }
            Some(ExprToken::Str(s)) => {
                let s = s.clone();
                self.pos += 1;
                Ok(ExprValue::Str(s))
            }
            _ => Err(Unevaluatable),
        }
    }
}

/// Equality in `%if`: numeric when both sides are numbers, string
/// equality otherwise.
fn values_equal(a: &ExprValue, b: &ExprValue) -> bool {
    match (a, b) {
        (ExprValue::Int(x), ExprValue::Int(y)) => x == y,
        (ExprValue::Str(x), ExprValue::Str(y)) => x == y,
        (ExprValue::Int(x), ExprValue::Str(y)) | (ExprValue::Str(y), ExprValue::Int(x)) => {
            y.parse::<i64>().is_ok_and(|n| n == *x)
        }
    }
}

/// Where a finding or skip refers to, for message text: the
/// own-config line when known, else the project the line came from.
/// A buildconfig line number is never shown: it refers to
/// server-concatenated text the user never sees.
fn line_or_project(own_line: Option<usize>, project: &str) -> String {
    match own_line {
        Some(n) => format!("line {n}"),
        None => format!("project {project}"),
    }
}

/// The solver state accumulated while parsing one buildconfig: the
/// list contents with per-token provenance, and the substitute map.
#[derive(Debug, Clone, Default)]
struct SolverState {
    lists: HashMap<&'static str, Vec<ListToken>>,
    /// The same lists as sets, kept in sync: membership tests are
    /// O(1) instead of a scan per token.
    sets: HashMap<&'static str, HashSet<String>>,
    known: HashMap<&'static str, bool>,
    substitute: HashMap<String, SubstEntry>,
    substitute_known: bool,
}

/// One token in a solver list, with the project and own-config line
/// that put it there.
#[derive(Debug, Clone)]
struct ListToken {
    token: String,
    project: String,
    own_line: Option<usize>,
}

/// One entry in the substitute map, with the line that set it.
#[derive(Debug, Clone)]
struct SubstEntry {
    values: Vec<String>,
    project: String,
    own_line: Option<usize>,
    /// The key was a macro-free literal on the line that set it:
    /// an override of it by a later unconditional line with a
    /// literal key is proven independent of the repository path.
    key_literal: bool,
}

impl SolverState {
    fn new() -> Self {
        let mut state = SolverState {
            lists: HashMap::new(),
            sets: HashMap::new(),
            known: HashMap::new(),
            substitute: HashMap::new(),
            substitute_known: true,
        };
        for directive in LIST_DIRECTIVES {
            state.lists.insert(directive, Vec::new());
            state.sets.insert(directive, HashSet::new());
            state.known.insert(directive, true);
        }
        state
    }

    fn list_known(&self, keyword: &str) -> bool {
        self.known.get(keyword).copied().unwrap_or(true)
    }

    /// Whether the list holds `token`, by the set: O(1).
    fn list_contains(&self, keyword: &str, token: &str) -> bool {
        self.sets
            .get(keyword)
            .is_some_and(|set| set.contains(token))
    }

    /// The list and its set for a list directive, creating them when
    /// a future keyword change leaves one unseeded. `None` when the
    /// keyword is not a list directive at all.
    fn list_and_set(
        &mut self,
        keyword: &str,
    ) -> Option<(&mut Vec<ListToken>, &mut HashSet<String>)> {
        let key = LIST_DIRECTIVES.iter().find(|k| **k == keyword).copied()?;
        Some((
            self.lists.entry(key).or_default(),
            self.sets.entry(key).or_default(),
        ))
    }

    /// Add a token to a list, keeping the set in sync.
    fn list_push(&mut self, keyword: &str, token: ListToken) {
        if let Some((list, set)) = self.list_and_set(keyword) {
            set.insert(token.token.clone());
            list.push(token);
        }
    }

    /// Remove a token from a list, keeping the set in sync.
    fn list_remove(&mut self, keyword: &str, token: &str) {
        if let Some((list, set)) = self.list_and_set(keyword) {
            set.remove(token);
            list.retain(|t| t.token != token);
        }
    }

    /// Clear a list, keeping the set in sync.
    fn list_clear(&mut self, keyword: &str) {
        if let Some((list, set)) = self.list_and_set(keyword) {
            set.clear();
            list.clear();
        }
    }

    /// Whether a list is empty.
    fn list_is_empty(&self, keyword: &str) -> bool {
        self.lists.get(keyword).is_some_and(|list| list.is_empty())
    }

    fn poison(&mut self, keyword: &str) {
        if let Some(static_key) = LIST_DIRECTIVES.iter().find(|k| **k == keyword) {
            self.known.insert(static_key, false);
        }
    }
}

/// The state of one `%if` segment in an evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SegmentState {
    /// The segment ran; its lines are live.
    Active,
    /// The segment never ran because its condition was false and no
    /// earlier segment ran; its lines contributed nothing.
    Dead,
    /// The segment's condition could not be evaluated, or an enclosing
    /// segment was uncertain; its lines may or may not have run.
    Uncertain,
}

/// One closed segment of a `%if` group.
#[derive(Debug, Clone)]
struct SegmentEffect {
    /// The `%if` / `%elif` / `%else` line (buildconfig line).
    marker_line: usize,
    /// The marker line in the own config, when the group is in the
    /// own config.
    marker_own_line: Option<usize>,
    /// The first content line (buildconfig line); greater than
    /// `content_end` when the segment is empty.
    content_start: usize,
    /// The last content line (buildconfig line).
    content_end: usize,
    /// The content lines in the own config, when the group is in the
    /// own config. Filled in by `finish_groups`.
    content_own_start: Option<usize>,
    content_own_end: Option<usize>,
    state: SegmentState,
    first: bool,
}

/// One closed `%if` group.
#[derive(Debug, Clone)]
struct GroupEffect {
    if_own_line: Option<usize>,
    endif_own_line: Option<usize>,
    /// The project the group came from.
    project: String,
    /// The buildconfig line of the `### from` marker starting the
    /// section the group is in (0 when the text has no markers):
    /// a segment's marker offset from it identifies the branch
    /// across repositories.
    section_base: usize,
    /// Which section of the buildconfig the group is in (0 for the
    /// text before the first marker, then one per `### from`
    /// marker): two sections of one project can hold a branch at
    /// the same offset from their marker, so the offset alone does
    /// not identify the branch.
    section_ordinal: usize,
    segments: Vec<SegmentEffect>,
}

/// What one (repository, architecture) evaluation proves about the
/// own config: the lines and tokens that are dead in this evaluation.
#[derive(Debug, Clone, Default)]
struct EvalPlan {
    /// Own lines whose whole line is dead.
    dead_lines: HashSet<usize>,
    /// (own line, token) pairs where the token is a duplicate, with
    /// the number of duplicate occurrences on that line.
    duplicate_tokens: HashMap<(usize, String), usize>,
    /// (own line, token) pairs where the token is a dead removal
    /// (`!X` of a never-added token) or a dead clear (`!*` on an
    /// empty list), with the number of dead occurrences on that
    /// line. Applied per token: the line itself goes only when no
    /// token is left.
    dead_tokens: HashMap<(usize, String), usize>,
    /// Groups keyed by the `%if` line in the own config.
    dead_groups: HashMap<usize, GroupDead>,
    /// Own lines in `dead_lines` whose removal is proven independent
    /// of the repository path and architecture: an overridden
    /// `Substitute` line whose overriding line is unconditional and
    /// whose keys are macro-free literals. A cleanup without the
    /// path-dependent opt-in removes only these lines.
    independent_dead_lines: HashSet<usize>,
    /// Same-line duplicate tokens: (own line, token) pairs where a
    /// duplicate occurrence is proven by the line itself, so the
    /// removal does not depend on the repository path or the
    /// architecture. Counts take the minimum across evaluations,
    /// like `duplicate_tokens`.
    independent_duplicates: HashMap<(usize, String), usize>,
    /// `%if` branches dead in this evaluation, keyed by (project,
    /// section ordinal, marker line offset from the section
    /// marker): the identity is stable across repositories, so the
    /// intersection over evaluations proves a branch dead in every
    /// evaluation. A section without a marker has no identity: its
    /// offset is an absolute buildconfig line, so two different
    /// branches in different repositories could share it.
    dead_branch_ids: HashSet<(String, usize, usize)>,
}

impl EvalPlan {
    /// The plan restricted to proofs independent of the repository
    /// path and architecture: same-line duplicate tokens and
    /// unconditionally overridden `Substitute` lines. Dead removals,
    /// dead clears and dead branches depend on what the inherited
    /// configs and the evaluated architectures contribute, so a
    /// downstream project that inherits this config can evaluate
    /// them live; they need the explicit opt-in.
    fn restricted(&self) -> EvalPlan {
        EvalPlan {
            dead_lines: self
                .dead_lines
                .intersection(&self.independent_dead_lines)
                .copied()
                .collect(),
            duplicate_tokens: self.independent_duplicates.clone(),
            dead_tokens: HashMap::new(),
            dead_groups: HashMap::new(),
            independent_dead_lines: self.independent_dead_lines.clone(),
            independent_duplicates: self.independent_duplicates.clone(),
            dead_branch_ids: HashSet::new(),
        }
    }
}

/// The dead segments of one `%if` group in one evaluation.
#[derive(Debug, Clone, Default)]
struct GroupDead {
    /// Dead segments keyed by marker own-line: (content own start,
    /// content own end, whether the segment is the first of its
    /// group).
    dead_segments: HashMap<usize, (Option<usize>, Option<usize>, bool)>,
    /// Every segment of the group is dead.
    all_dead: bool,
    endif_own_line: Option<usize>,
}

/// The outcome of evaluating one buildconfig for one architecture.
#[derive(Debug)]
struct EvalOutcome {
    plan: EvalPlan,
    state: SolverState,
    findings: Vec<PrjconfFinding>,
    skipped: Vec<PrjconfSkipped>,
}

/// One `%if` frame on the conditional stack.
struct IfFrame {
    parent_active: bool,
    parent_uncertain: bool,
    taken: bool,
    active: bool,
    uncertain: bool,
    if_own_line: Option<usize>,
    if_project: String,
    /// The buildconfig line of the `### from` marker starting the
    /// section the group is in (0 when the text has no markers).
    section_base: usize,
    /// Which section of the buildconfig the group is in; see
    /// `GroupEffect::section_ordinal`.
    section_ordinal: usize,
    segments: Vec<SegmentEffect>,
    current_marker_line: usize,
    current_marker_own_line: Option<usize>,
    current_content_start: usize,
    current_first: bool,
}

impl IfFrame {
    /// Close the current segment with the given state and start a new
    /// one at `marker_line`. The new segment's content starts after
    /// the marker's last line (`marker_last_line`), not its first:
    /// a continued marker (`%elif 0 == 1 \` plus its continuation)
    /// would otherwise lend its continuation line to the next
    /// segment's content range.
    fn close_segment(
        &mut self,
        state: SegmentState,
        marker_line: usize,
        marker_last_line: usize,
        marker_own_line: Option<usize>,
    ) {
        let content_end = marker_line.saturating_sub(1);
        self.segments.push(SegmentEffect {
            marker_line: self.current_marker_line,
            marker_own_line: self.current_marker_own_line,
            content_start: self.current_content_start,
            content_end,
            content_own_start: None,
            content_own_end: None,
            state,
            first: self.current_first,
        });
        self.current_marker_line = marker_line;
        self.current_marker_own_line = marker_own_line;
        self.current_content_start = marker_last_line + 1;
        self.current_first = false;
    }

    /// The state of the current segment: active, dead, or uncertain.
    fn current_state(&self) -> SegmentState {
        if self.uncertain {
            SegmentState::Uncertain
        } else if self.active {
            SegmentState::Active
        } else {
            SegmentState::Dead
        }
    }
}

/// One logical line of a buildconfig: physical lines joined by a
/// trailing `\`, with the first and last physical line numbers.
/// Findings use the first line; a segment's content starts after
/// the last line of its marker.
struct LogicalLine {
    first: usize,
    last: usize,
    text: String,
}

/// Join the physical lines of `text` into logical lines: a line
/// ending in `\` continues on the next line, the backslash dropped.
/// obs-build (`Build::Rpm`, `parse`) joins `%define` values and
/// multi-line `%{?x:` conditions this way; without the join a
/// continuation line holding `Prefer: x` would parse as a live
/// directive inside the macro body.
fn logical_lines(text: &str) -> Vec<LogicalLine> {
    let mut out: Vec<LogicalLine> = Vec::new();
    let mut current: Option<LogicalLine> = None;
    for (idx, physical) in text.lines().enumerate() {
        let lineno = idx + 1;
        let (part, continues) = match physical.strip_suffix('\\') {
            Some(rest) => (rest, true),
            None => (physical, false),
        };
        match &mut current {
            Some(line) => {
                line.text.push_str(part);
                line.last = lineno;
            }
            None => {
                current = Some(LogicalLine {
                    first: lineno,
                    last: lineno,
                    text: part.to_string(),
                });
            }
        }
        if !continues {
            // `current` is always open here: the match above either
            // extended the open line or opened a new one.
            if let Some(line) = current.take() {
                out.push(line);
            }
        }
    }
    if let Some(line) = current {
        out.push(line);
    }
    out
}

/// The standard macros `Build::read_config` seeds before parsing
/// (obs-build `Build.pm`, `$std_macros`), plus `_target_cpu` (the
/// repository architecture) and `_target_os` (`linux`). Without them,
/// `%ifarch %{ix86}` and `%if %{_target_cpu} == ...` — common idioms —
/// go unevaluatable and whole segments go uncertain.
fn std_macros(arch: &str) -> MacroTable {
    let mut table = MacroTable::default();
    for (name, value) in [
        ("nil", ""),
        ("ix86", "i386 i486 i586 i686 pentium3 pentium4 athlon geode"),
        ("x86_64", "x86_64 x86_64_v2 x86_64_v3 x86_64_v4 amd64 em64t"),
        (
            "arm32",
            "armv3l armv4b armv4l armv4tl armv5tl armv5tel armv5tejl armv6l armv6hl armv7l armv7hl armv7hnl armv8l armv8hl armv8hnl armv8hcnl",
        ),
        ("arm", "%{arm32}"),
        (
            "arml",
            "armv3l armv4l armv5l armv5tel armv6l armv6hl armv7l armv7hl armv7hnl",
        ),
        ("armb", "armv4b armv5b armv5teb"),
        ("arm64", "aarch64"),
        ("mips32", "mips mipsel mipsr6 mipsr6el"),
        ("mips64", "mips64 mips64el mips64r6 mips64r6el"),
        ("mipseb", "mips mipsr6 mips64 mips64r6"),
        ("mipsel", "mipsel mipsr6el mips64el mips64r6el"),
        ("mips", "%{mips32} %{mips64}"),
        ("loongarch64", "loongarch64"),
        ("sparc", "sparc sparcv8 sparcv9 sparcv9v sparc64 sparc64v"),
        ("alpha", "alpha alphaev56 alphaev6 alphaev67"),
        ("power64", "ppc64 ppc64p7 ppc64le"),
        ("riscv32", "riscv32"),
        ("riscv64", "riscv64"),
        ("riscv128", "riscv128"),
        ("riscv", "%{riscv32} %{riscv64} %{riscv128}"),
    ] {
        table.defs.insert(name.to_string(), value.to_string());
    }
    table
        .defs
        .insert("_target_cpu".to_string(), arch.to_string());
    table
        .defs
        .insert("_target_os".to_string(), "linux".to_string());
    table
}

/// Evaluate one buildconfig for one architecture.
///
/// `line_map` maps buildconfig lines in the own section to lines in
/// the project's own `_config`; `repository` names the repository
/// being evaluated (for findings); `own_project` is the project
/// being validated.
fn evaluate_buildconfig(
    text: &str,
    arch: &str,
    repository: &str,
    own_project: &str,
    line_map: &HashMap<usize, usize>,
) -> EvalOutcome {
    let mut macros = std_macros(arch);
    let mut state = SolverState::new();
    let mut frames: Vec<IfFrame> = Vec::new();
    let mut in_macros_section = false;
    let mut macros_buffer: Vec<String> = Vec::new();
    let mut section_project = own_project.to_string();
    let mut section_base = 0;
    let mut section_ordinal = 0;
    let mut seen_content = false;
    let mut plan = EvalPlan::default();
    let mut findings: Vec<PrjconfFinding> = Vec::new();
    let mut skipped: Vec<PrjconfSkipped> = Vec::new();
    let mut groups: Vec<GroupEffect> = Vec::new();

    let lines = logical_lines(text);
    let line_count = lines.len();

    // The active/uncertain state of the current position: active when
    // every enclosing frame's current segment is active; uncertain
    // when any enclosing segment is uncertain.
    let position = |frames: &[IfFrame]| -> (bool, bool) {
        let mut active = true;
        let mut uncertain = false;
        for frame in frames {
            if !frame.active {
                active = false;
            }
            if frame.uncertain {
                uncertain = true;
            }
        }
        (active, uncertain)
    };

    let mut i = 0;
    while i < line_count {
        let logical = &lines[i];
        let lineno = logical.first;
        let last_lineno = logical.last;
        let raw = logical.text.as_str();
        i += 1;
        let line = raw.trim_end();
        let own_line = line_map.get(&lineno).copied();
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        if !in_macros_section {
            if let Some(marker) = trimmed.strip_prefix("### from ") {
                let project = marker.split_whitespace().next().unwrap_or("");
                if !project.is_empty() {
                    section_project = project.to_string();
                    section_base = lineno;
                    section_ordinal += 1;
                }
                seen_content = false;
                continue;
            }
        }
        if trimmed.starts_with('#') {
            continue;
        }
        if is_injected_buildconfig_line(line) {
            // A server header line: it defines a macro, but it is
            // not content of the config (a `FromScratch:` after it is
            // still the first content line).
            apply_macro_def_line(line, &mut macros, true);
            continue;
        }
        if in_macros_section {
            if trimmed.eq_ignore_ascii_case(":macros") {
                in_macros_section = false;
                macros_buffer.clear();
            } else {
                macros_buffer.push(line.to_string());
            }
            continue;
        }
        if trimmed.eq_ignore_ascii_case("macros:") {
            in_macros_section = true;
            macros_buffer.clear();
            seen_content = true;
            continue;
        }

        // Conditional directives are processed even inside inactive
        // segments, so nesting stays balanced. The words match
        // exactly, lowercase: the server's `Build::Rpm::parse` is
        // case-sensitive, so `%IF` / `%Endif` are not conditionals
        // here either.
        if let Some(after_pct) = trimmed.strip_prefix('%') {
            let word = after_pct.split_whitespace().next().unwrap_or("");
            match word {
                "elifnarch" | "elifnos" => {
                    // Not conditionals (`Build::Rpm::parse` handles
                    // only `%elif` / `%elifarch` / `%elifos`): the
                    // server ignores the line, so this does too.
                    continue;
                }
                "if" | "ifarch" | "ifnarch" | "ifos" | "ifnos" | "elif" | "elifarch" | "elifos"
                | "else" | "endif" => {
                    handle_conditional(
                        trimmed,
                        word,
                        lineno,
                        last_lineno,
                        own_line,
                        &section_project,
                        section_base,
                        section_ordinal,
                        arch,
                        &macros,
                        &mut frames,
                        &mut groups,
                        &mut findings,
                        &mut skipped,
                        repository,
                    );
                    // A `%` line is not content for the
                    // `FromScratch:` rule: `read_config` never sees
                    // it (the `%if` lines are consumed by
                    // `Build::Rpm::parse`).
                    continue;
                }
                _ => {}
            }
        }

        let (active, uncertain) = position(&frames);
        // A line inside a dead segment contributes nothing and cannot
        // be evaluated; a line inside an uncertain segment is skipped
        // for application but poisons what it touches.
        if !active && !uncertain {
            continue;
        }

        // A line counts as content for the `FromScratch:` rule
        // when `read_config` sees it: not a `%` line (conditionals,
        // `%define`, macro references — consumed by
        // `Build::Rpm::parse` or skipped as `%` lines) and not
        // `FromScratch:` itself (`read_config` `next`s past it
        // without marking content, so consecutive `FromScratch:`
        // lines are all fine).
        let is_content = !trimmed.starts_with('%')
            && !trimmed
                .split_whitespace()
                .next()
                .is_some_and(|w| w.eq_ignore_ascii_case("fromscratch:"));
        process_line(
            line,
            own_line,
            &section_project,
            repository,
            seen_content,
            active,
            uncertain,
            !frames.is_empty(),
            &mut macros,
            &mut state,
            &mut plan,
            &mut findings,
            &mut skipped,
            0,
        );
        if is_content {
            seen_content = true;
        }
    }

    // An unclosed `macros:` section runs to the end of the text; the
    // scheduler strips it, so its lines define nothing for the solver.
    // (The trailing `Macros:` section the server appends is always
    // unclosed.)
    for frame in &frames {
        findings.push(PrjconfFinding {
            kind: PrjconfFindingKind::UnclosedIf,
            severity: PrjconfSeverity::Error,
            repository: Some(repository.to_string()),
            project: frame.if_project.clone(),
            line: frame.if_own_line,
            message: format!(
                "%if at {} is never closed",
                line_or_project(frame.if_own_line, &frame.if_project)
            ),
        });
    }

    // The final solver state feeds the resolution and shadowing
    // checks, run by the caller.
    finish_groups(groups, line_map, &mut plan, &mut findings, repository, arch);
    EvalOutcome {
        plan,
        state,
        findings,
        skipped,
    }
}

/// Record the completed groups into the plan and report their dead
/// branches as findings.
fn finish_groups(
    groups: Vec<GroupEffect>,
    line_map: &HashMap<usize, usize>,
    plan: &mut EvalPlan,
    findings: &mut Vec<PrjconfFinding>,
    repository: &str,
    arch: &str,
) {
    for mut group in groups {
        // Map each segment's content span to own-config lines, for
        // the dead-segment removals in the cleanup.
        for segment in &mut group.segments {
            let mut own_lines: Vec<usize> = Vec::new();
            if segment.content_start <= segment.content_end {
                for l in segment.content_start..=segment.content_end {
                    if let Some(own) = line_map.get(&l) {
                        own_lines.push(*own);
                    }
                }
            }
            segment.content_own_start = own_lines.iter().min().copied();
            segment.content_own_end = own_lines.iter().max().copied();
        }
        for segment in &group.segments {
            if segment.state != SegmentState::Dead {
                continue;
            }
            // The branch identity for the dead-in-every-evaluation
            // proof: the marker's offset from the section marker is
            // stable across repositories, unlike its buildconfig
            // line number.
            // A section without a marker has no cross-repository
            // identity: its offset is an absolute buildconfig line,
            // so different branches in different repositories could
            // share one id and produce a spurious `DeadBranch`
            // warning in `validate`.
            if group.section_base != 0 {
                plan.dead_branch_ids.insert((
                    group.project.clone(),
                    group.section_ordinal,
                    segment.marker_line.saturating_sub(group.section_base),
                ));
            }
            findings.push(PrjconfFinding {
                kind: PrjconfFindingKind::DeadBranch,
                severity: PrjconfSeverity::Warning,
                repository: Some(repository.to_string()),
                project: group.project.clone(),
                line: segment.marker_own_line,
                message: format!(
                    "%if branch starting at {} is false for repository {repository} on architecture {arch}",
                    line_or_project(segment.marker_own_line, &group.project)
                ),
            });
        }
        let Some(if_own) = group.if_own_line else {
            // A dead branch in an inherited config is a finding, not
            // a cleanup: the line is not ours to remove.
            continue;
        };
        let mut dead = GroupDead {
            dead_segments: HashMap::new(),
            all_dead: !group.segments.is_empty()
                && group.segments.iter().all(|s| s.state == SegmentState::Dead),
            endif_own_line: group.endif_own_line,
        };
        for segment in &group.segments {
            if segment.state != SegmentState::Dead {
                continue;
            }
            if let Some(marker_own) = segment.marker_own_line {
                dead.dead_segments.insert(
                    marker_own,
                    (
                        segment.content_own_start,
                        segment.content_own_end,
                        segment.first,
                    ),
                );
            }
        }
        if !dead.dead_segments.is_empty() || dead.all_dead {
            plan.dead_groups.insert(if_own, dead);
        }
    }
}

/// Split a string into its first whitespace-separated word and the
/// rest after it (leading whitespace trimmed).
fn split_first_word(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    match s.find(char::is_whitespace) {
        Some(idx) => (&s[..idx], s[idx..].trim_start()),
        None => (s, ""),
    }
}

/// Parse a macro definition line (`%define name value`, `%global
/// name value`, or `%name value`) and apply it to the table. A
/// definition in an active segment defines the macro; in an
/// uncertain segment the name's value becomes unknowable. Returns
/// the defined name, when the line is a definition.
fn apply_macro_def_line(line: &str, macros: &mut MacroTable, active: bool) -> Option<String> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix('%')?;
    let (word, after) = split_first_word(rest);
    let (name, value) = if word == "define" || word == "global" {
        let (name, value) = split_first_word(after);
        (name.to_string(), value.to_string())
    } else {
        (word.to_string(), after.to_string())
    };
    if name.is_empty() {
        return None;
    }
    if name.contains('(') {
        // A parameterised macro: its value depends on the call site,
        // so any use is unevaluatable.
        let base = name.split('(').next().unwrap_or(&name).to_string();
        macros.unknown.insert(base.clone());
        return Some(base);
    }
    if active {
        macros.defs.insert(name.clone(), value);
        macros.unknown.remove(&name);
    } else {
        macros.defs.remove(&name);
        macros.unknown.insert(name.clone());
    }
    Some(name)
}

/// Handle one `%if` / `%ifarch` / `%ifnarch` / `%ifos` / `%ifnos` /
/// `%elif` / `%elifarch` / `%elifos` / `%else` / `%endif` line.
/// Conditions are evaluated only when the enclosing context is
/// active and certain; otherwise the new segment inherits the
/// parent's state. An unevaluatable condition makes its segment
/// uncertain, and every later segment of the same group stays
/// uncertain too: whether it runs depends on a condition whose
/// outcome is unknown.
#[allow(clippy::too_many_arguments)]
fn handle_conditional(
    trimmed: &str,
    word: &str,
    lineno: usize,
    last_lineno: usize,
    own_line: Option<usize>,
    section_project: &str,
    section_base: usize,
    section_ordinal: usize,
    arch: &str,
    macros: &MacroTable,
    frames: &mut Vec<IfFrame>,
    groups: &mut Vec<GroupEffect>,
    findings: &mut Vec<PrjconfFinding>,
    skipped: &mut Vec<PrjconfSkipped>,
    repository: &str,
) {
    let parent_active = frames.last().map(|f| f.active).unwrap_or(true);
    let parent_uncertain = frames.iter().any(|f| f.uncertain);
    match word {
        "if" | "ifarch" | "ifnarch" | "ifos" | "ifnos" => {
            let (active, uncertain) = if parent_uncertain {
                (false, true)
            } else if !parent_active {
                (false, false)
            } else {
                let (_, expr) = split_first_word(&trimmed[1..]);
                match eval_if_directive(word, expr, arch, macros) {
                    Ok(result) => (result, false),
                    Err(_) => {
                        skipped.push(PrjconfSkipped {
                            repository: Some(repository.to_string()),
                            project: section_project.to_string(),
                            line: own_line,
                            reason: format!(
                                "%if condition at {} could not be evaluated; its branches were not evaluated",
                                line_or_project(own_line, section_project)
                            ),
                        });
                        (false, true)
                    }
                }
            };
            frames.push(IfFrame {
                parent_active,
                parent_uncertain,
                taken: active,
                active,
                uncertain,
                if_own_line: own_line,
                if_project: section_project.to_string(),
                section_base,
                section_ordinal,
                segments: Vec::new(),
                current_marker_line: lineno,
                current_marker_own_line: own_line,
                current_content_start: last_lineno + 1,
                current_first: true,
            });
        }
        "elif" | "elifarch" | "elifos" => {
            let Some(frame) = frames.last_mut() else {
                findings.push(PrjconfFinding {
                    kind: PrjconfFindingKind::ParseError,
                    severity: PrjconfSeverity::Error,
                    repository: Some(repository.to_string()),
                    project: section_project.to_string(),
                    line: own_line,
                    message: format!(
                        "%{word} at {} has no matching %if",
                        line_or_project(own_line, section_project)
                    ),
                });
                return;
            };
            let state = frame.current_state();
            frame.close_segment(state, lineno, last_lineno, own_line);
            let (active, uncertain) = if frame.parent_uncertain {
                (false, true)
            } else if !frame.parent_active {
                (false, false)
            } else if frame.uncertain {
                // An earlier condition could not be evaluated, so
                // whether this branch runs is unknown — it is
                // uncertain, never definitely active.
                (false, true)
            } else if frame.taken {
                (false, false)
            } else {
                let (_, expr) = split_first_word(&trimmed[1..]);
                let eval_word = match word {
                    "elifarch" => "ifarch",
                    "elifos" => "ifos",
                    _ => "if",
                };
                match eval_if_directive(eval_word, expr, arch, macros) {
                    Ok(result) => (result, false),
                    Err(_) => {
                        skipped.push(PrjconfSkipped {
                            repository: Some(repository.to_string()),
                            project: section_project.to_string(),
                            line: own_line,
                            reason: format!(
                                "%{word} condition at {} could not be evaluated; its branch was not evaluated",
                                line_or_project(own_line, section_project)
                            ),
                        });
                        (false, true)
                    }
                }
            };
            frame.taken |= active;
            frame.active = active;
            frame.uncertain = uncertain;
        }
        "else" => {
            let Some(frame) = frames.last_mut() else {
                findings.push(PrjconfFinding {
                    kind: PrjconfFindingKind::ParseError,
                    severity: PrjconfSeverity::Error,
                    repository: Some(repository.to_string()),
                    project: section_project.to_string(),
                    line: own_line,
                    message: format!(
                        "%else at {} has no matching %if",
                        line_or_project(own_line, section_project)
                    ),
                });
                return;
            };
            let state = frame.current_state();
            frame.close_segment(state, lineno, last_lineno, own_line);
            let (active, uncertain) = if frame.parent_uncertain {
                (false, true)
            } else if !frame.parent_active {
                (false, false)
            } else if frame.uncertain {
                (false, true)
            } else {
                (!frame.taken, false)
            };
            frame.taken |= active;
            frame.active = active;
            frame.uncertain = uncertain;
        }
        "endif" => {
            let Some(frame) = frames.pop() else {
                findings.push(PrjconfFinding {
                    kind: PrjconfFindingKind::ParseError,
                    severity: PrjconfSeverity::Error,
                    repository: Some(repository.to_string()),
                    project: section_project.to_string(),
                    line: own_line,
                    message: format!(
                        "%endif at {} has no matching %if",
                        line_or_project(own_line, section_project)
                    ),
                });
                return;
            };
            let mut frame = frame;
            let state = frame.current_state();
            frame.close_segment(state, lineno, last_lineno, own_line);
            groups.push(GroupEffect {
                if_own_line: frame.if_own_line,
                endif_own_line: own_line,
                project: frame.if_project.clone(),
                section_base: frame.section_base,
                section_ordinal: frame.section_ordinal,
                segments: frame.segments,
            });
        }
        _ => unreachable!(),
    }
}

/// Evaluate the condition of a `%if` / `%ifarch` / `%ifnarch` /
/// `%ifos` / `%ifnos` directive. `%ifarch` / `%ifnarch` match the
/// architecture against the (macro-expanded) list after the
/// keyword; `%ifos` / `%ifnos` match `_target_os` (`linux` unless
/// the config redefines it) the same way.
fn eval_if_directive(
    word: &str,
    expr: &str,
    arch: &str,
    macros: &MacroTable,
) -> Result<bool, Unevaluatable> {
    match word {
        "if" => eval_condition(expr, macros),
        "ifarch" | "ifnarch" => {
            let expanded = expand_macros(expr, macros, 0)?;
            let listed = expanded.split_whitespace().any(|a| a == arch);
            Ok(if word == "ifarch" { listed } else { !listed })
        }
        "ifos" | "ifnos" => {
            let expanded = expand_macros(expr, macros, 0)?;
            let os = macros
                .defs
                .get("_target_os")
                .map(String::as_str)
                .unwrap_or("linux");
            let listed = expanded.split_whitespace().any(|o| o == os);
            Ok(if word == "ifos" { listed } else { !listed })
        }
        _ => Err(Unevaluatable),
    }
}

/// Process one non-conditional line of a buildconfig. `active` is
/// false inside a dead or uncertain segment; `uncertain` is true
/// inside an uncertain segment. A line in an uncertain segment is
/// never applied, but it poisons the lists it could touch.
#[allow(clippy::too_many_arguments)]
fn process_line(
    line: &str,
    own_line: Option<usize>,
    section_project: &str,
    repository: &str,
    seen_content: bool,
    active: bool,
    uncertain: bool,
    in_conditional: bool,
    macros: &mut MacroTable,
    state: &mut SolverState,
    plan: &mut EvalPlan,
    findings: &mut Vec<PrjconfFinding>,
    skipped: &mut Vec<PrjconfSkipped>,
    depth: usize,
) {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix('%') {
        let (word, _) = split_first_word(rest);
        // Exact lowercase, like the conditional dispatch above:
        // `%DEFINE` does not define a macro and `%UNDEF` does not
        // undefine one.
        match word {
            "define" | "global" => {
                apply_macro_def_line(line, macros, active);
                return;
            }
            "undef" => {
                let (_, after) = split_first_word(rest);
                let (name, _) = split_first_word(after);
                if active && !name.is_empty() {
                    macros.defs.remove(name);
                    macros.unknown.remove(name);
                } else if uncertain && !name.is_empty() {
                    macros.defs.remove(name);
                    macros.unknown.insert(name.to_string());
                }
                return;
            }
            _ => {
                // `%name value` defines macro `name`; a bare `%name`
                // (or `%{...}`) line expands and is re-processed.
                let (_, after) = split_first_word(rest);
                if after.is_empty() {
                    if depth >= 4 {
                        return;
                    }
                    match expand_macros(trimmed, macros, 0) {
                        Ok(expanded) => {
                            let expanded = expanded.trim();
                            if expanded.is_empty() || expanded == trimmed {
                                return;
                            }
                            process_line(
                                expanded,
                                own_line,
                                section_project,
                                repository,
                                seen_content,
                                active,
                                uncertain,
                                in_conditional,
                                macros,
                                state,
                                plan,
                                findings,
                                skipped,
                                depth + 1,
                            );
                        }
                        Err(_) => {
                            skipped.push(PrjconfSkipped {
                                repository: Some(repository.to_string()),
                                project: section_project.to_string(),
                                line: own_line,
                                reason: format!(
                                    "macro reference at {} could not be expanded",
                                    line_or_project(own_line, section_project)
                                ),
                            });
                        }
                    }
                } else {
                    apply_macro_def_line(line, macros, active);
                }
                return;
            }
        }
    }

    let (first, _) = split_first_word(trimmed);
    let Some(keyword_raw) = first.strip_suffix(':') else {
        findings.push(PrjconfFinding {
            kind: PrjconfFindingKind::UnknownKeyword,
            severity: PrjconfSeverity::Warning,
            repository: Some(repository.to_string()),
            project: section_project.to_string(),
            line: own_line,
            message: format!(
                "unknown keyword `{first}` at {}",
                line_or_project(own_line, section_project)
            ),
        });
        return;
    };
    let keyword = keyword_raw.to_ascii_lowercase();
    // A trailing `#...` comment contributes no tokens: the first
    // token starting with `#` begins it. (The server tokenizes the
    // comment words, but they are never packages, so the checks
    // skip them — and the rewrite below preserves the comment.)
    let raw_tokens: Vec<&str> = trimmed
        .split_whitespace()
        .skip(1)
        .take_while(|t| !t.starts_with('#'))
        .collect();

    if LIST_DIRECTIVES.contains(&keyword.as_str()) {
        process_list_directive(
            &keyword,
            &raw_tokens,
            own_line,
            section_project,
            repository,
            active,
            macros,
            state,
            plan,
            findings,
            skipped,
        );
        return;
    }
    match keyword.as_str() {
        "substitute" => {
            process_substitute(
                &raw_tokens,
                own_line,
                section_project,
                repository,
                active,
                in_conditional,
                macros,
                state,
                plan,
                findings,
                skipped,
            );
        }
        "fromscratch" => {
            if seen_content {
                findings.push(PrjconfFinding {
                    kind: PrjconfFindingKind::ParseError,
                    severity: PrjconfSeverity::Error,
                    repository: Some(repository.to_string()),
                    project: section_project.to_string(),
                    line: own_line,
                    message: format!(
                        "FromScratch: at {} is not the first content line",
                        line_or_project(own_line, section_project)
                    ),
                });
            }
        }
        "target" | "changetarget" => {
            // `read_config` redefines `_target_cpu` from the
            // target's first `-`-separated part.
            let mut value = String::new();
            let mut unexpandable = false;
            for raw in raw_tokens {
                match expand_macros(raw, macros, 0) {
                    Ok(expanded) => {
                        if !value.is_empty() {
                            value.push(' ');
                        }
                        value.push_str(&expanded);
                    }
                    Err(_) => {
                        unexpandable = true;
                        break;
                    }
                }
            }
            if unexpandable {
                macros.defs.remove("_target_cpu");
                macros.unknown.insert("_target_cpu".to_string());
            } else {
                let cpu = value.split('-').next().unwrap_or("").to_string();
                if cpu.is_empty() {
                    macros.defs.remove("_target_cpu");
                    macros.unknown.insert("_target_cpu".to_string());
                } else if active {
                    macros.defs.insert("_target_cpu".to_string(), cpu);
                    macros.unknown.remove("_target_cpu");
                } else if uncertain {
                    macros.defs.remove("_target_cpu");
                    macros.unknown.insert("_target_cpu".to_string());
                }
            }
        }
        "distmacro" => {
            let value = trimmed[first.len()..].trim_start();
            if let Some(def) = value.strip_prefix("%define ") {
                let def_line = format!("%define {def}");
                apply_macro_def_line(&def_line, macros, active);
            }
        }
        _ if OTHER_DIRECTIVES.contains(&keyword.as_str()) => {
            // A recognised directive with no solver-list effect.
        }
        _ => {
            findings.push(PrjconfFinding {
                kind: PrjconfFindingKind::UnknownKeyword,
                severity: PrjconfSeverity::Warning,
                repository: Some(repository.to_string()),
                project: section_project.to_string(),
                line: own_line,
                message: format!(
                    "unknown keyword `{keyword}:` at {}",
                    line_or_project(own_line, section_project)
                ),
            });
        }
    }
}

/// Apply one list directive line (`Prefer:`, `Support:`, ...).
/// Tokens are macro-expanded first; a token that cannot be expanded
/// makes the line unevaluatable, so it is skipped and the list is
/// poisoned.
#[allow(clippy::too_many_arguments)]
fn process_list_directive(
    keyword: &str,
    raw_tokens: &[&str],
    own_line: Option<usize>,
    section_project: &str,
    repository: &str,
    active: bool,
    macros: &MacroTable,
    state: &mut SolverState,
    plan: &mut EvalPlan,
    findings: &mut Vec<PrjconfFinding>,
    skipped: &mut Vec<PrjconfSkipped>,
) {
    let mut tokens: Vec<String> = Vec::new();
    for raw in raw_tokens {
        match expand_macros(raw, macros, 0) {
            Ok(expanded) => tokens.extend(expanded.split_whitespace().map(str::to_string)),
            Err(_) => {
                skipped.push(PrjconfSkipped {
                    repository: Some(repository.to_string()),
                    project: section_project.to_string(),
                    line: own_line,
                    reason: format!(
                        "token in `{keyword}:` at {} could not be expanded; the line was not evaluated",
                        line_or_project(own_line, section_project)
                    ),
                });
                if active {
                    state.poison(keyword);
                }
                return;
            }
        }
    }
    if !active {
        state.poison(keyword);
        return;
    }
    let known = state.list_known(keyword);
    // The tokens already seen on this line: a duplicate proven by
    // the line itself is independent of the repository path and the
    // architecture — both occurrences live or die with the line.
    let mut line_seen: HashSet<String> = HashSet::new();
    for token in &tokens {
        let same_line = !line_seen.insert(token.clone());
        if token == "!*" {
            if known && state.list_is_empty(keyword) {
                findings.push(PrjconfFinding {
                    kind: PrjconfFindingKind::DeadClear,
                    severity: PrjconfSeverity::Warning,
                    repository: Some(repository.to_string()),
                    project: section_project.to_string(),
                    line: own_line,
                    message: format!(
                        "`{keyword}: !*` at {} clears a list that is already empty",
                        line_or_project(own_line, section_project)
                    ),
                });
                if let Some(own) = own_line {
                    *plan.dead_tokens.entry((own, token.clone())).or_insert(0) += 1;
                }
            }
            state.list_clear(keyword);
        } else if let Some(removed) = token.strip_prefix('!') {
            if known && !state.list_contains(keyword, removed) {
                findings.push(PrjconfFinding {
                    kind: PrjconfFindingKind::DeadRemoval,
                    severity: PrjconfSeverity::Warning,
                    repository: Some(repository.to_string()),
                    project: section_project.to_string(),
                    line: own_line,
                    message: format!(
                        "`{keyword}: !{removed}` at {} removes a token that was never added",
                        line_or_project(own_line, section_project)
                    ),
                });
                if let Some(own) = own_line {
                    *plan.dead_tokens.entry((own, token.clone())).or_insert(0) += 1;
                }
            }
            state.list_remove(keyword, removed);
        } else if known && state.list_contains(keyword, token) {
            findings.push(PrjconfFinding {
                kind: PrjconfFindingKind::DuplicateToken,
                severity: PrjconfSeverity::Warning,
                repository: Some(repository.to_string()),
                project: section_project.to_string(),
                line: own_line,
                message: format!(
                    "`{token}` in `{keyword}:` at {} is already in the list",
                    line_or_project(own_line, section_project)
                ),
            });
            if let Some(own) = own_line {
                *plan
                    .duplicate_tokens
                    .entry((own, token.clone()))
                    .or_insert(0) += 1;
                if same_line {
                    *plan
                        .independent_duplicates
                        .entry((own, token.clone()))
                        .or_insert(0) += 1;
                }
            }
        } else {
            state.list_push(
                keyword,
                ListToken {
                    token: token.clone(),
                    project: section_project.to_string(),
                    own_line,
                },
            );
        }
    }
}

/// Apply one `Substitute:` line. The map is last-wins per key; the
/// earlier line for a key contributes nothing once a later line sets
/// the same key.
#[allow(clippy::too_many_arguments)]
fn process_substitute(
    raw_tokens: &[&str],
    own_line: Option<usize>,
    section_project: &str,
    repository: &str,
    active: bool,
    in_conditional: bool,
    macros: &MacroTable,
    state: &mut SolverState,
    plan: &mut EvalPlan,
    findings: &mut Vec<PrjconfFinding>,
    skipped: &mut Vec<PrjconfSkipped>,
) {
    let mut tokens: Vec<String> = Vec::new();
    for raw in raw_tokens {
        match expand_macros(raw, macros, 0) {
            Ok(expanded) => tokens.extend(expanded.split_whitespace().map(str::to_string)),
            Err(_) => {
                skipped.push(PrjconfSkipped {
                    repository: Some(repository.to_string()),
                    project: section_project.to_string(),
                    line: own_line,
                    reason: format!(
                        "token in `Substitute:` at {} could not be expanded; the line was not evaluated",
                        line_or_project(own_line, section_project)
                    ),
                });
                if active {
                    state.substitute_known = false;
                }
                return;
            }
        }
    }
    if tokens.is_empty() {
        return;
    }
    if !active {
        state.substitute_known = false;
        return;
    }
    let key = tokens[0].clone();
    if key == "!*" {
        if state.substitute_known && state.substitute.is_empty() {
            findings.push(PrjconfFinding {
                kind: PrjconfFindingKind::DeadClear,
                severity: PrjconfSeverity::Warning,
                repository: Some(repository.to_string()),
                project: section_project.to_string(),
                line: own_line,
                message: format!(
                    "`Substitute: !*` at {} clears a map that is already empty",
                    line_or_project(own_line, section_project)
                ),
            });
            if let Some(own) = own_line {
                plan.dead_lines.insert(own);
            }
        }
        state.substitute.clear();
        return;
    }
    if let Some(removed) = key.strip_prefix('!') {
        if state.substitute_known && !state.substitute.contains_key(removed) {
            findings.push(PrjconfFinding {
                kind: PrjconfFindingKind::DeadRemoval,
                severity: PrjconfSeverity::Warning,
                repository: Some(repository.to_string()),
                project: section_project.to_string(),
                line: own_line,
                message: format!(
                    "`Substitute: !{removed}` at {} removes a key that was never set",
                    line_or_project(own_line, section_project)
                ),
            });
            if let Some(own) = own_line {
                plan.dead_lines.insert(own);
            }
        }
        state.substitute.remove(removed);
        return;
    }
    let values = tokens[1..].to_vec();
    if let Some(earlier) = state.substitute.get(&key) {
        if state.substitute_known {
            findings.push(PrjconfFinding {
                kind: PrjconfFindingKind::OverriddenSubstitute,
                severity: PrjconfSeverity::Warning,
                repository: Some(repository.to_string()),
                project: earlier.project.clone(),
                line: earlier.own_line,
                message: format!(
                    "`Substitute: {key}` at {} is overridden by {}",
                    line_or_project(earlier.own_line, &earlier.project),
                    line_or_project(own_line, section_project)
                ),
            });
            if let Some(own) = earlier.own_line {
                plan.dead_lines.insert(own);
                // The override is proven independent of the
                // repository path only when the overriding line
                // always runs (no enclosing `%if`) and both keys
                // are macro-free literals: an inherited macro
                // could otherwise rename either key downstream.
                if !in_conditional && earlier.key_literal && !raw_tokens[0].contains('%') {
                    plan.independent_dead_lines.insert(own);
                }
            }
        }
    }
    state.substitute.insert(
        key,
        SubstEntry {
            values,
            project: section_project.to_string(),
            own_line,
            key_literal: !raw_tokens[0].contains('%'),
        },
    );
}

/// A line in a buildconfig that comes from the server, not from any
/// project's config: a `### from` marker, a `#!!line` marker, or one
/// of the `%define _project` / `_repository` / `_is_this_project` /
/// `_is_in_project` / `_obs_feature_exclude_cpu_constraints` headers
/// the concatenation prepends per config.
fn is_injected_buildconfig_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("### from ")
        || trimmed.starts_with("#!!line ")
        || trimmed.starts_with("%define _repository ")
        || trimmed.starts_with("%define _project ")
        || trimmed.starts_with("%define _is_this_project ")
        || trimmed.starts_with("%define _is_in_project ")
        || trimmed.starts_with("%define _obs_feature_exclude_cpu_constraints ")
}

/// The lines of an own config as they appear in the buildconfig's own
/// section, per the server's concatenation: the text up to the first
/// `:macros` line stays in place (with a flushed `Macros:` block
/// inserted before it, handled by the caller); a `macros:` section
/// after that, or an unclosed one, is relocated to the trailing
/// section and does not appear. Each line carries its own-config line
/// number.
fn expected_own_section_lines(own_config: &str) -> Vec<(usize, &str)> {
    let lines: Vec<(usize, &str)> = own_config
        .lines()
        .enumerate()
        .map(|(idx, line)| (idx + 1, line))
        .collect();
    let first_close = lines
        .iter()
        .position(|(_, l)| l.trim().eq_ignore_ascii_case(":macros"));
    match first_close {
        Some(close_idx) => {
            let mut expected: Vec<(usize, &str)> = lines[..=close_idx].to_vec();
            let rest = &lines[close_idx + 1..];
            let next_macros = rest
                .iter()
                .position(|(_, l)| l.trim().eq_ignore_ascii_case("macros:"));
            match next_macros {
                Some(m) => expected.extend_from_slice(&rest[..m]),
                None => expected.extend_from_slice(rest),
            }
            expected
        }
        None => match lines
            .iter()
            .position(|(_, l)| l.trim().eq_ignore_ascii_case("macros:"))
        {
            Some(m) => lines[..m].to_vec(),
            None => lines,
        },
    }
}

/// Whether the lines after the own config's lines form the trailing
/// `Macros:` section the server appends: an optional blank line,
/// `Macros:`, then only macro content (`### from` markers, `macros:`/
/// `:macros` markers, `%` macro lines, blanks) to the end.
fn is_trailing_macros_section(lines: &[&str]) -> bool {
    let mut lines = lines.iter().peekable();
    while lines.peek().is_some_and(|l| l.trim().is_empty()) {
        lines.next();
    }
    if lines.next() != Some(&"Macros:") {
        return false;
    }
    for line in lines {
        let trimmed = line.trim_start();
        if trimmed.is_empty()
            || trimmed.starts_with("### from ")
            || trimmed.eq_ignore_ascii_case("macros:")
            || trimmed.eq_ignore_ascii_case(":macros")
            || trimmed.starts_with('%')
        {
            continue;
        }
        return false;
    }
    true
}

/// Map the buildconfig's own section back to the project's own
/// `_config`: buildconfig line -> own-config line. The own section is
/// the own config verbatim minus its relocated `Macros:` sections,
/// plus the injected header lines and a flushed `Macros:` block when
/// the config holds a `:macros` line, and the trailing `Macros:`
/// section follows it. The mapping succeeds only when the own
/// config's expected lines match the section's lines exactly; a
/// mismatch means the server's text does not line up with the own
/// config, so no line number can be proven and a cleanup is refused.
fn map_buildconfig_to_own(
    buildconfig: &str,
    own_config: &str,
    own_project: &str,
) -> Option<HashMap<usize, usize>> {
    let bc_lines: Vec<&str> = buildconfig.lines().collect();
    // Find the own section: the last section whose project is the own
    // project (the own project is last in the concatenation). A
    // `### from` line inside a `Macros:` section (a flushed block or
    // the trailing section) is macro content, not a boundary.
    let mut own_start: Option<usize> = None;
    let mut own_end = bc_lines.len();
    let mut in_macros = false;
    for (idx, line) in bc_lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.eq_ignore_ascii_case("macros:") {
            in_macros = true;
            continue;
        }
        if trimmed.eq_ignore_ascii_case(":macros") {
            in_macros = false;
            continue;
        }
        if in_macros {
            continue;
        }
        if let Some(marker) = trimmed.strip_prefix("### from ") {
            let project = marker.split_whitespace().next().unwrap_or("");
            if !project.is_empty() {
                if project == own_project {
                    own_start = Some(idx + 1);
                    own_end = bc_lines.len();
                } else if own_start.is_some() {
                    own_end = idx;
                    break;
                }
            }
        }
    }
    let (start, end) = match own_start {
        Some(start) => (start, own_end),
        None => {
            // No provenance markers: the whole text is one section,
            // attributed to the own project.
            if bc_lines
                .iter()
                .any(|l| l.trim_start().starts_with("### from "))
            {
                // Markers exist but none names the own project: the
                // own config is not in this buildconfig.
                return Some(HashMap::new());
            }
            (0, bc_lines.len())
        }
    };
    let section: Vec<(usize, &str)> = bc_lines[start..end]
        .iter()
        .enumerate()
        .map(|(off, line)| (start + off + 1, *line))
        .filter(|(_, line)| !is_injected_buildconfig_line(line))
        .collect();
    let expected = expected_own_section_lines(own_config);
    // Try the section as-is; when the config holds a `:macros` line
    // the server flushes a `Macros:` ... `:Macros` block before the
    // config's own lines, so also try with that block stripped.
    if let Some(map) = try_map_section(&section, &expected) {
        return Some(map);
    }
    if let Some(stripped) = strip_flushed_block(&section) {
        if let Some(map) = try_map_section(&stripped, &expected) {
            return Some(map);
        }
    }
    None
}

/// Try to map one candidate section slice to the own config's
/// expected lines: the expected lines must match as a prefix, and
/// the remainder must be the trailing `Macros:` section (or empty).
fn try_map_section(
    section: &[(usize, &str)],
    expected: &[(usize, &str)],
) -> Option<HashMap<usize, usize>> {
    if section.len() < expected.len() {
        return None;
    }
    for ((_, bc), (_, own)) in section.iter().zip(expected.iter()) {
        if bc.trim_end() != own.trim_end() {
            return None;
        }
    }
    let remainder: Vec<&str> = section[expected.len()..].iter().map(|(_, l)| *l).collect();
    if !remainder.is_empty() && !is_trailing_macros_section(&remainder) {
        return None;
    }
    Some(
        section[..expected.len()]
            .iter()
            .zip(expected.iter())
            .map(|((bc_line, _), (own_line, _))| (*bc_line, *own_line))
            .collect(),
    )
}

/// Strip a leading flushed `Macros:` ... `:Macros` block (plus the
/// blank line the server emits before it and after it). The flush's
/// closing line is `:Macros` exactly as emitted; a case-insensitive
/// match is the fallback. Returns `None` when no flushed block is
/// present.
fn strip_flushed_block<'a>(section: &'a [(usize, &'a str)]) -> Option<Vec<(usize, &'a str)>> {
    let first = section.iter().position(|(_, l)| !l.trim().is_empty())?;
    if section[first].1.trim() != "Macros:" {
        return None;
    }
    let close = section
        .iter()
        .skip(first)
        .position(|(_, l)| l.trim() == ":Macros")
        .map(|p| p + first)
        .or_else(|| {
            section
                .iter()
                .skip(first)
                .position(|(_, l)| l.trim().eq_ignore_ascii_case(":macros"))
                .map(|p| p + first)
        })?;
    let mut rest: Vec<(usize, &str)> = section[close + 1..].to_vec();
    let skip = rest
        .iter()
        .position(|(_, l)| !l.trim().is_empty())
        .unwrap_or(rest.len());
    rest.drain(..skip);
    Some(rest)
}

/// The resolution data for one repository: which names resolve to a
/// package in a project on the repo's path, or to a binary built by
/// one. A `None` package set means the project's package list could
/// not be read, so names in it cannot be proven unresolvable.
struct ResolutionContext<'a> {
    path_projects: &'a HashSet<String>,
    packages: &'a HashMap<String, Option<HashSet<String>>>,
    owners: &'a HashMap<String, Option<Vec<Owner>>>,
}

/// Whether `name` resolves in this repository: a package in a project
/// on the path, or a binary owned by a project on the path. `None`
/// means the check could not be run (a package list or an owner
/// lookup failed), so the name is left alone.
fn resolves(name: &str, ctx: &ResolutionContext<'_>) -> Option<bool> {
    let mut unknown_project = false;
    for project in ctx.path_projects {
        match ctx.packages.get(project) {
            Some(Some(packages)) => {
                if packages.contains(name) {
                    return Some(true);
                }
            }
            Some(None) => unknown_project = true,
            None => {}
        }
    }
    if unknown_project {
        return None;
    }
    match ctx.owners.get(name) {
        Some(Some(owners)) => Some(
            owners
                .iter()
                .any(|owner| ctx.path_projects.contains(&owner.project)),
        ),
        Some(None) => None,
        None => Some(false),
    }
}

/// The full validation of one project, with the per-evaluation plans
/// a cleanup needs.
struct FullValidation {
    validation: PrjconfValidation,
    plans: Vec<EvalPlan>,
    own_config: String,
    mapping_failed: bool,
}

impl ObsClient<'_> {
    /// Read a project's raw prjconf (`GET /source/<project>/_config`).
    /// An absent config reads as an empty string, not a 404.
    pub fn project_config(&self, project: &str) -> Result<String, GeectlError> {
        if !valid_obs_name(project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {project:?}"),
            ));
        }
        if !self.has_auth() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot read a project config without OBS credentials (GEECTL_OBS_APIKEY[_FILE] or GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD[_FILE])",
            ));
        }
        let url = build_url(&self.base, &["source", project, "_config"], &[]);
        self.http.get_with(&url, &self.credentials)
    }

    /// Write a project's raw prjconf (`PUT /source/<project>/_config`).
    /// An empty body deletes the config. Scmsync-controlled projects
    /// refuse the write.
    pub fn put_project_config(&self, project: &str, config: &str) -> Result<(), GeectlError> {
        if !valid_obs_name(project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {project:?}"),
            ));
        }
        if !self.has_auth() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot write a project config without OBS credentials (GEECTL_OBS_APIKEY[_FILE] or GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD[_FILE])",
            ));
        }
        let url = build_url(&self.base, &["source", project, "_config"], &[]);
        let body = self.http.put(&url, config, &self.credentials)?;
        if parse_status_ok(&body) {
            Ok(())
        } else {
            let preview = crate::debug::format_body_preview(&body);
            let received = if preview.is_empty() {
                "<empty response>"
            } else {
                preview.as_str()
            };
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "unexpected OBS project config response for project {project}; received: {received}"
                ),
            ))
        }
    }

    /// Validate a project's prjconf across its repositories: every
    /// repository, every architecture. A `%if` branch is reported
    /// dead only when it is dead in every evaluation. Exits 24 at
    /// the CLI when error findings exist; warnings alone do not
    /// fail the run. One buildconfig fetch per repository; the
    /// package and owner lookups are cached across evaluations.
    /// Requires OBS credentials: without them the call fails closed
    /// before any request. At most 32 repositories are evaluated;
    /// a project with more is refused.
    pub fn validate_project_config(&self, project: &str) -> Result<PrjconfValidation, GeectlError> {
        Ok(self.validate_project_config_full(project)?.validation)
    }

    /// Clean a project's prjconf. Dry-run by default; `--live`
    /// applies the write. Refuses outright when the config has a
    /// parse error or when the buildconfig cannot be mapped back to
    /// the own config. Shares validate's credential gate and
    /// 32-repository bound.
    ///
    /// A prjconf is inherited: downstream projects concatenate this
    /// config with different repository paths, architectures and
    /// `_repository` / `_project` values. By default (`false`) the
    /// cleanup removes only lines proven dead independent of the
    /// repository path and architecture (same-line duplicate tokens
    /// and unconditionally overridden `Substitute` lines). Pass
    /// `true` to also remove lines proven dead in every repository
    /// and architecture evaluation of this project — the caller
    /// accepts that such a line can be live downstream.
    pub fn clean_project_config(
        &self,
        project: &str,
        dry_run: &DryRun,
        include_path_dependent: bool,
    ) -> Result<PrjconfCleanResult, GeectlError> {
        let full = self.validate_project_config_full(project)?;
        // Refuse when a parse error is in the own config, or when
        // an inherited `%if` is never closed: the unclosed frame
        // swallows the own config's lines, so the evaluation is
        // unreliable. A stray `%endif` / `%elif` / `%else` or a
        // `FromScratch:` in an inherited config does not corrupt
        // the own evaluation and does not refuse. The unclosed
        // `%if` is matched on its finding kind, never on the
        // message text: a reword must not silently drop the gate.
        // The refusal names the findings, in the same rendering
        // `validate` reports them in.
        if full.validation.findings.iter().any(|f| {
            f.severity == PrjconfSeverity::Error
                && (f.project.as_str() == project || f.kind == PrjconfFindingKind::UnclosedIf)
        }) {
            return Err(GeectlError::new(
                ErrorKind::ValidationFailed,
                format!(
                    "prjconf has parse errors; refusing to clean\n{}",
                    format_prjconf_report(&full.validation.findings, &full.validation.skipped)
                ),
            ));
        }
        if full.mapping_failed && !full.own_config.trim().is_empty() {
            // The server's buildconfig does not line up with the
            // own config, so no line number can be proven: a
            // server/config mismatch, not invalid input.
            return Err(GeectlError::new(
                ErrorKind::ValidationFailed,
                "could not map the buildconfig back to the own config; refusing to clean",
            ));
        }
        // A whitespace-only own config has nothing to clean:
        // no line exists to remove. Say so explicitly instead
        // of relying on the empty intersected plan to produce
        // the same no-op.
        let (cleaned, removed_lines) = if full.own_config.trim().is_empty() {
            (full.own_config.clone(), Vec::new())
        } else {
            let plan = intersect_plans(full.plans);
            let plan = if include_path_dependent {
                plan
            } else {
                plan.restricted()
            };
            apply_plan(&full.own_config, &plan)
        };
        let removed = removed_lines.len();
        let applied = if removed == 0 {
            false
        } else if dry_run.allow(&format!(
            "put cleaned config for {project} ({removed} line{} removed)",
            if removed == 1 { "" } else { "s" }
        )) {
            // The config was read once, then many sequential
            // fetches ran: re-read just before the write and refuse
            // on change, so an edit in between is never silently
            // overwritten.
            let current = self.project_config(project)?;
            if current != full.own_config {
                return Err(GeectlError::new(
                    ErrorKind::ValidationFailed,
                    "project config changed since it was read; refusing to clean",
                ));
            }
            self.put_project_config(project, &cleaned)?;
            true
        } else {
            false
        };
        Ok(PrjconfCleanResult {
            project: project.to_string(),
            applied,
            removed_lines,
            cleaned,
            findings: full.validation.findings,
            skipped: full.validation.skipped,
        })
    }

    fn validate_project_config_full(&self, project: &str) -> Result<FullValidation, GeectlError> {
        if !valid_obs_name(project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {project:?}"),
            ));
        }
        // The credential gate runs before any read: `project_meta`
        // is a public anonymous GET, and it must not precede the
        // fail-closed gate the way `project_config` enforces it.
        if !self.has_auth() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot validate a project config without OBS credentials (GEECTL_OBS_APIKEY[_FILE] or GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD[_FILE])",
            ));
        }
        let meta = self.project_meta(project)?;
        // Past the repository bound the call refuses before any
        // buildconfig fetch: a partial evaluation would hand `clean`
        // a plan proven dead only in the evaluated repositories.
        if meta.repositories.len() > MAX_PRJCONF_REPOSITORIES {
            return Err(GeectlError::new(
                ErrorKind::TooManyRepositories,
                format!(
                    "project {project} has {} repositories, more than the {} validate supports",
                    meta.repositories.len(),
                    MAX_PRJCONF_REPOSITORIES
                ),
            ));
        }
        let own_config = self.project_config(project)?;
        let mut findings: Vec<PrjconfFinding> = Vec::new();
        let mut skipped: Vec<PrjconfSkipped> = Vec::new();
        let mut plans: Vec<EvalPlan> = Vec::new();
        let mut mapping_failed = false;
        let mut package_cache: HashMap<String, Option<HashSet<String>>> = HashMap::new();
        let mut owner_cache: HashMap<String, Option<Vec<Owner>>> = HashMap::new();
        let mut owner_budget = OwnerLookupBudget::new();
        if meta.repositories.is_empty() {
            skipped.push(PrjconfSkipped {
                repository: None,
                project: project.to_string(),
                line: None,
                reason: "project has no repositories; nothing to evaluate the config against"
                    .to_string(),
            });
        }
        self.debug.emit(&format!(
            "prjconf validate: {project}: {} repositories, {} repository-architecture evaluations",
            meta.repositories.len(),
            meta.repositories
                .iter()
                .map(|repo| repo.arch.len())
                .sum::<usize>()
        ));
        for (index, repo) in meta.repositories.iter().enumerate() {
            self.debug.emit(&format!(
                "prjconf validate: {project}: repository {}/{} ({})",
                index + 1,
                meta.repositories.len(),
                sanitize_text_output(&repo.name)
            ));
            if repo.arch.is_empty() {
                skipped.push(PrjconfSkipped {
                    repository: Some(repo.name.clone()),
                    project: project.to_string(),
                    line: None,
                    reason:
                        "repository has no architectures; nothing to evaluate the config against"
                            .to_string(),
                });
                continue;
            }
            let buildconfig = self.buildconfig(project, &repo.name)?;
            let line_map = match map_buildconfig_to_own(&buildconfig, &own_config, project) {
                Some(map) => map,
                None => {
                    mapping_failed = true;
                    HashMap::new()
                }
            };
            let path_projects: HashSet<String> = repo
                .paths
                .iter()
                .map(|(p, _)| p.clone())
                .chain(std::iter::once(project.to_string()))
                .collect();
            for arch in &repo.arch {
                let mut outcome =
                    evaluate_buildconfig(&buildconfig, arch, &repo.name, project, &line_map);
                self.prjconf_resolve_and_shadow(
                    &mut outcome,
                    project,
                    &repo.name,
                    &path_projects,
                    &mut package_cache,
                    &mut owner_cache,
                    &mut owner_budget,
                );
                findings.append(&mut outcome.findings);
                skipped.append(&mut outcome.skipped);
                plans.push(outcome.plan);
            }
        }
        // A `%if` branch is reported dead only when it is dead in
        // every evaluation: a branch false in one repository or
        // architecture and live in another is a normal config, not
        // a finding. The per-evaluation findings are replaced by
        // one finding per branch in the intersected plan: own
        // branches name their line, inherited branches name their
        // project (their line numbers refer to
        // server-concatenated text the user never sees).
        findings.retain(|f| f.kind != PrjconfFindingKind::DeadBranch);
        let intersected = intersect_plans(plans.clone());
        for group in intersected.dead_groups.values() {
            for marker_own in group.dead_segments.keys() {
                findings.push(PrjconfFinding {
                    kind: PrjconfFindingKind::DeadBranch,
                    severity: PrjconfSeverity::Warning,
                    repository: None,
                    project: project.to_string(),
                    line: Some(*marker_own),
                    message: format!(
                        "%if branch starting at line {marker_own} is false in every repository and architecture evaluation"
                    ),
                });
            }
        }
        for (branch_project, _, _) in &intersected.dead_branch_ids {
            if branch_project == project {
                continue;
            }
            findings.push(PrjconfFinding {
                kind: PrjconfFindingKind::DeadBranch,
                severity: PrjconfSeverity::Warning,
                repository: None,
                project: branch_project.clone(),
                line: None,
                message: format!(
                    "%if branch in project {branch_project} is false in every repository and architecture evaluation"
                ),
            });
        }
        dedup_findings(&mut findings);
        dedup_skipped(&mut skipped);
        Ok(FullValidation {
            validation: PrjconfValidation {
                project: project.to_string(),
                findings,
                skipped,
            },
            plans,
            own_config,
            mapping_failed,
        })
    }

    /// The resolution and shadowing checks for one (repository,
    /// architecture) evaluation, run over the evaluation's final
    /// solver state. The shared owner-lookup budget is an eighth
    /// argument; the alternative is a context struct for one call.
    #[allow(clippy::too_many_arguments)]
    fn prjconf_resolve_and_shadow(
        &self,
        outcome: &mut EvalOutcome,
        project: &str,
        repository: &str,
        path_projects: &HashSet<String>,
        package_cache: &mut HashMap<String, Option<HashSet<String>>>,
        owner_cache: &mut HashMap<String, Option<Vec<Owner>>>,
        owner_budget: &mut OwnerLookupBudget,
    ) {
        // The names the check needs, in first-seen order.
        let mut names: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let remember = |name: &str, names: &mut Vec<String>, seen: &mut HashSet<String>| {
            let name = name.strip_prefix('-').unwrap_or(name);
            if looks_like_package(name) && seen.insert(name.to_string()) {
                names.push(name.to_string());
            }
        };
        for keyword in PACKAGE_LISTS {
            if let Some(tokens) = outcome.state.lists.get(keyword) {
                for token in tokens {
                    remember(&token.token, &mut names, &mut seen);
                }
            }
        }
        for entry in outcome.state.substitute.values() {
            for value in &entry.values {
                remember(value, &mut names, &mut seen);
            }
        }
        for key in outcome.state.substitute.keys() {
            remember(key, &mut names, &mut seen);
        }
        // Fetch the package lists the check needs. A list that cannot
        // be read leaves the project unknown, never empty: an empty
        // list would falsely prove names unresolvable.
        for path_project in path_projects {
            if !package_cache.contains_key(path_project) {
                match self.list_packages(path_project) {
                    Ok(packages) => {
                        package_cache
                            .insert(path_project.clone(), Some(packages.into_iter().collect()));
                    }
                    Err(_) => {
                        package_cache.insert(path_project.clone(), None);
                        outcome.skipped.push(PrjconfSkipped {
                            repository: Some(repository.to_string()),
                            project: project.to_string(),
                            line: None,
                            reason: format!(
                                "package list for path project `{path_project}` could not be read; names in it were left alone"
                            ),
                        });
                    }
                }
            }
        }
        let any_unknown_project = path_projects
            .iter()
            .any(|p| package_cache.get(p) == Some(&None));
        for name in &names {
            if owner_cache.contains_key(name) {
                continue;
            }
            // A name that resolves to a package needs no owner lookup;
            // a name in an unreadable project cannot be proven either
            // way, so no lookup runs for it.
            let mut resolves_by_package = false;
            for path_project in path_projects {
                if let Some(Some(packages)) = package_cache.get(path_project) {
                    if packages.contains(name) {
                        resolves_by_package = true;
                        break;
                    }
                }
            }
            if resolves_by_package || any_unknown_project {
                continue;
            }
            if owner_budget.remaining == 0 {
                // Past the cap the name is left unresolved, never
                // guessed: `None` in the cache means the check
                // could not run, so the name is left alone.
                owner_cache.insert(name.clone(), None);
                if !owner_budget.reported {
                    owner_budget.reported = true;
                    outcome.skipped.push(PrjconfSkipped {
                        repository: Some(repository.to_string()),
                        project: project.to_string(),
                        line: None,
                        reason: format!(
                            "owner lookups capped at {MAX_PRJCONF_OWNER_LOOKUPS}; further names were left unresolved"
                        ),
                    });
                }
                continue;
            }
            owner_budget.remaining -= 1;
            match self.owners_for_binary(name) {
                Ok(owners) => {
                    owner_cache.insert(name.clone(), Some(owners));
                }
                Err(_) => {
                    owner_cache.insert(name.clone(), None);
                    outcome.skipped.push(PrjconfSkipped {
                        repository: Some(repository.to_string()),
                        project: project.to_string(),
                        line: None,
                        reason: format!(
                            "binary owner lookup for `{name}` failed; the name was left alone"
                        ),
                    });
                }
            }
        }
        let ctx = ResolutionContext {
            path_projects,
            packages: package_cache,
            owners: owner_cache,
        };
        // Unresolvable targets in the solver package lists.
        for keyword in PACKAGE_LISTS {
            if let Some(tokens) = outcome.state.lists.get(keyword) {
                for token in tokens {
                    let name = token.token.strip_prefix('-').unwrap_or(&token.token);
                    // `Ignore:` / `Conflict:` tokens split on `[,:]`
                    // upstream (`init_helper_hashes`): the first part
                    // is the package, the rest scopes it. Resolve the
                    // package part, not the whole token.
                    let name = if *keyword == "ignore" || *keyword == "conflict" {
                        name.split([':', ',']).next().unwrap_or(name)
                    } else {
                        name
                    };
                    if name.is_empty() || !looks_like_package(name) {
                        continue;
                    }
                    match resolves(name, &ctx) {
                        Some(true) | None => {}
                        Some(false) => {
                            outcome.findings.push(PrjconfFinding {
                                kind: PrjconfFindingKind::UnresolvableTarget,
                                severity: PrjconfSeverity::Warning,
                                repository: Some(repository.to_string()),
                                project: token.project.clone(),
                                line: token.own_line,
                                message: format!(
                                    "`{name}` in `{keyword}:` resolves to no package in any repository on the path and no binary built by one"
                                ),
                            });
                        }
                    }
                }
            }
        }
        // Unresolvable substitute keys and values.
        for (key, entry) in &outcome.state.substitute {
            for name in std::iter::once(key.as_str()).chain(entry.values.iter().map(String::as_str))
            {
                if !looks_like_package(name) {
                    continue;
                }
                if let Some(false) = resolves(name, &ctx) {
                    outcome.findings.push(PrjconfFinding {
                        kind: PrjconfFindingKind::UnresolvableTarget,
                        severity: PrjconfSeverity::Warning,
                        repository: Some(repository.to_string()),
                        project: entry.project.clone(),
                        line: entry.own_line,
                        message: format!(
                            "`{name}` in `Substitute:` resolves to no package in any repository on the path and no binary built by one"
                        ),
                    });
                }
            }
        }
        // A `Prefer: -X` avoid while Support / Preinstall / Required
        // still carries X.
        let mut carried: HashSet<&str> = HashSet::new();
        for keyword in ["support", "preinstall", "required"] {
            if let Some(tokens) = outcome.state.lists.get(keyword) {
                for token in tokens {
                    carried.insert(token.token.as_str());
                }
            }
        }
        if let Some(prefer) = outcome.state.lists.get("prefer") {
            for token in prefer {
                if let Some(avoided) = token.token.strip_prefix('-') {
                    if carried.contains(avoided) {
                        outcome.findings.push(PrjconfFinding {
                            kind: PrjconfFindingKind::ShadowedAvoid,
                            severity: PrjconfSeverity::Warning,
                            repository: Some(repository.to_string()),
                            project: token.project.clone(),
                            line: token.own_line,
                            message: format!(
                                "`Prefer: -{avoided}` loses to an inherited `Support` / `Preinstall` / `Required` carrying `{avoided}`"
                            ),
                        });
                    }
                }
            }
        }
    }
}

/// Render one finding as a text line: `repository/project:line:
/// kind: message` (the repository and the line are omitted when
/// unknown). Server-supplied strings are sanitized: control
/// characters are escaped, so a crafted config cannot inject
/// terminal escapes into the output, and overlong tokens are cut.
pub fn prjconf_finding_line(finding: &PrjconfFinding) -> String {
    sanitize_text_output(&format!(
        "{}: {}: {}",
        finding_location(&finding.repository, &finding.project, finding.line),
        finding.kind.as_str(),
        finding.message
    ))
}

/// Render one skipped item as a text line, sanitized like a finding.
pub fn prjconf_skipped_line(skipped: &PrjconfSkipped) -> String {
    sanitize_text_output(&format!(
        "{}: skipped: {}",
        finding_location(&skipped.repository, &skipped.project, skipped.line),
        skipped.reason
    ))
}

fn finding_location(repository: &Option<String>, project: &str, line: Option<usize>) -> String {
    match (repository, line) {
        (Some(repo), Some(line)) => format!(
            "{}/{}:{}",
            sanitize_text_output(repo),
            sanitize_text_output(project),
            line
        ),
        (Some(repo), None) => format!(
            "{}/{}",
            sanitize_text_output(repo),
            sanitize_text_output(project)
        ),
        (None, Some(line)) => format!("{}:{}", sanitize_text_output(project), line),
        (None, None) => sanitize_text_output(project),
    }
}

/// The findings and skipped items of a validation as the text the
/// CLI reports: `N finding(s):` followed by one line per finding
/// and skipped item. Both `validate` and the `clean` refusals use
/// this one rendering, so a refusal names what it refused on.
pub fn format_prjconf_report(findings: &[PrjconfFinding], skipped: &[PrjconfSkipped]) -> String {
    let mut lines: Vec<String> = findings.iter().map(prjconf_finding_line).collect();
    for skip in skipped {
        lines.push(prjconf_skipped_line(skip));
    }
    format!("{} finding(s):\n{}", findings.len(), lines.join("\n"))
}

/// Make a string safe for text output: control characters are
/// escaped (`\u{...}`), and a line longer than the cap is cut with
/// a note. JSON output keeps the raw strings (JSON escaping already
/// covers it); this is only for the human-readable rendering.
pub fn sanitize_text_output(text: &str) -> String {
    const MAX_TEXT_LINE: usize = 500;
    let mut out = String::with_capacity(text.len());
    let mut count = 0;
    for c in text.chars() {
        if count >= MAX_TEXT_LINE {
            out.push_str("… (truncated)");
            break;
        }
        if c.is_control() {
            let escaped = format!("\\u{{{:x}}}", c as u32);
            count += escaped.chars().count();
            out.push_str(&escaped);
        } else {
            count += 1;
            out.push(c);
        }
    }
    out
}

/// A finding's identity for deduplication.
type FindingKey = (
    PrjconfFindingKind,
    Option<String>,
    String,
    Option<usize>,
    String,
);

/// Remove duplicate findings (same kind, repository, project, line
/// and message) and sort for stable output.
fn dedup_findings(findings: &mut Vec<PrjconfFinding>) {
    let mut seen: HashSet<FindingKey> = HashSet::new();
    findings.retain(|f| {
        seen.insert((
            f.kind,
            f.repository.clone(),
            f.project.clone(),
            f.line,
            f.message.clone(),
        ))
    });
    findings.sort_by_cached_key(|f| {
        (
            f.repository.clone().unwrap_or_default(),
            f.project.clone(),
            f.line.unwrap_or(usize::MAX),
            f.message.clone(),
        )
    });
}

/// Remove duplicate skipped entries and sort for stable output.
fn dedup_skipped(skipped: &mut Vec<PrjconfSkipped>) {
    let mut seen: HashSet<(Option<String>, String, Option<usize>, String)> = HashSet::new();
    skipped.retain(|s| {
        seen.insert((
            s.repository.clone(),
            s.project.clone(),
            s.line,
            s.reason.clone(),
        ))
    });
    skipped.sort_by_cached_key(|s| {
        (
            s.repository.clone().unwrap_or_default(),
            s.project.clone(),
            s.line.unwrap_or(usize::MAX),
            s.reason.clone(),
        )
    });
}

/// Intersect the per-evaluation plans: a line or token is removable
/// only when it is dead in every evaluation. Duplicate-token and
/// dead-token counts take the minimum across evaluations, so a token
/// live in any evaluation is never removed.
fn intersect_plans(plans: Vec<EvalPlan>) -> EvalPlan {
    let mut iter = plans.into_iter();
    let Some(first) = iter.next() else {
        return EvalPlan::default();
    };
    let mut acc = first;
    for plan in iter {
        acc.dead_lines.retain(|line| plan.dead_lines.contains(line));
        acc.duplicate_tokens
            .retain(|key, count| match plan.duplicate_tokens.get(key) {
                Some(other) => {
                    *count = (*count).min(*other);
                    true
                }
                None => false,
            });
        acc.dead_tokens
            .retain(|key, count| match plan.dead_tokens.get(key) {
                Some(other) => {
                    *count = (*count).min(*other);
                    true
                }
                None => false,
            });
        acc.dead_groups.retain(|if_line, group| {
            let Some(other) = plan.dead_groups.get(if_line) else {
                return false;
            };
            group
                .dead_segments
                .retain(|marker, _| other.dead_segments.contains_key(marker));
            group.all_dead = group.all_dead && other.all_dead;
            !group.dead_segments.is_empty() || group.all_dead
        });
        acc.independent_dead_lines
            .retain(|line| plan.independent_dead_lines.contains(line));
        acc.independent_duplicates.retain(|key, count| {
            match plan.independent_duplicates.get(key) {
                Some(other) => {
                    *count = (*count).min(*other);
                    true
                }
                None => false,
            }
        });
        acc.dead_branch_ids
            .retain(|id| plan.dead_branch_ids.contains(id));
    }
    acc
}

/// A physical line of a text: byte ranges for its content (without
/// the terminator) and for the whole line (with it).
struct RawLine {
    start: usize,
    content_end: usize,
    end: usize,
}

/// Split a text into physical lines with byte spans, preserving
/// terminators (`\n` or `\r\n`). The line count matches
/// `str::lines` for the same text.
fn raw_lines(text: &str) -> Vec<RawLine> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            let content_end = if i > start && bytes[i - 1] == b'\r' {
                i - 1
            } else {
                i
            };
            lines.push(RawLine {
                start,
                content_end,
                end: i + 1,
            });
            i += 1;
            start = i;
        } else {
            i += 1;
        }
    }
    if start < bytes.len() {
        lines.push(RawLine {
            start,
            content_end: bytes.len(),
            end: bytes.len(),
        });
    }
    lines
}

/// The physical lines one logical line spans: a line whose content
/// ends in `\` continues on the next line. Returns (first, last)
/// 0-based line indices.
fn logical_spans(lines: &[RawLine], text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut first = 0;
    for (idx, line) in lines.iter().enumerate() {
        if !text[line.start..line.content_end].ends_with('\\') {
            spans.push((first, idx));
            first = idx + 1;
        }
    }
    if first < lines.len() {
        spans.push((first, lines.len() - 1));
    }
    spans
}

/// The effective text of a logical line (continuation backslashes
/// and line terminators removed) with, for each of its bytes, the
/// raw byte offset it came from.
fn logical_effective(
    text: &str,
    lines: &[RawLine],
    first: usize,
    last: usize,
) -> (String, Vec<usize>) {
    let mut effective = String::new();
    let mut raw_offsets: Vec<usize> = Vec::new();
    for (offset, line) in lines[first..=last].iter().enumerate() {
        // A continuation drops its trailing backslash; the last
        // line of the span keeps its content whole, unless it ends
        // the file mid-continuation: `logical_lines` drops that
        // backslash too, so keeping it here would hand the token
        // edits a stray `\` word on the final line.
        let mut content_end = line.content_end;
        if first + offset < last || text[line.start..content_end].ends_with('\\') {
            content_end -= 1;
        }
        effective.push_str(&text[line.start..content_end]);
        raw_offsets.extend(line.start..content_end);
    }
    (effective, raw_offsets)
}

/// The words of a line (maximal non-whitespace runs) with byte
/// ranges, and the byte offset of the trailing comment when the
/// line has one: the first word starting with `#` begins it.
fn line_words(effective: &str) -> (Vec<(usize, usize)>, Option<usize>) {
    let mut words = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in effective.char_indices() {
        if c.is_whitespace() {
            if let Some(s) = start.take() {
                if effective[s..].starts_with('#') {
                    return (words, Some(s));
                }
                words.push((s, i));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        if effective[s..].starts_with('#') {
            return (words, Some(s));
        }
        words.push((s, effective.len()));
    }
    (words, None)
}

/// Mark a whole logical line for deletion: its byte span, and every
/// physical line it covers.
fn delete_logical_span(
    lines: &[RawLine],
    first: usize,
    last: usize,
    delete: &mut Vec<(usize, usize)>,
    removed: &mut HashSet<usize>,
) {
    delete.push((lines[first].start, lines[last].end));
    for n in first + 1..=last + 1 {
        removed.insert(n);
    }
}
///
/// The rewrite splices byte ranges out of the original text: line
/// terminators are preserved (`\r\n` stays `\r\n`), and a token
/// edit removes only the token and one adjacent run of whitespace,
/// so alignment and tab layout of the surviving tokens is kept.
/// A line that continues onto the next line (`\` at the end) is one
/// logical line: removing it removes the whole span, and a token
/// edit finds the token anywhere in the span.
fn apply_plan(own_config: &str, plan: &EvalPlan) -> (String, Vec<usize>) {
    let lines = raw_lines(own_config);
    let spans = logical_spans(&lines, own_config);
    let span_by_first: HashMap<usize, (usize, usize)> = spans
        .iter()
        .map(|&(first, last)| (first + 1, (first, last)))
        .collect();
    // Byte ranges to delete, and the physical lines already gone
    // (a token edit never touches one of those).
    let mut delete: Vec<(usize, usize)> = Vec::new();
    let mut removed: HashSet<usize> = HashSet::new();
    for line in &plan.dead_lines {
        if let Some(&(first, last)) = span_by_first.get(line) {
            delete_logical_span(&lines, first, last, &mut delete, &mut removed);
        } else if let Some(raw) = lines.get(line - 1) {
            delete.push((raw.start, raw.end));
            removed.insert(*line);
        }
    }
    // Group removals: a wholly dead group goes entirely; otherwise
    // each dead segment loses its marker and content, except a dead
    // first segment, which keeps its `%if` marker (a later segment
    // still needs it) and loses only its content.
    for (if_line, group) in &plan.dead_groups {
        if group.all_dead {
            if let Some(endif_line) = group.endif_own_line {
                // The whole group goes, marker lines included: use
                // the logical spans, so a `%if` line that continues
                // onto the next line takes its continuation with it.
                if let (Some(&(if_first, _)), Some(&(_, endif_last))) =
                    (span_by_first.get(if_line), span_by_first.get(&endif_line))
                {
                    delete.push((lines[if_first].start, lines[endif_last].end));
                    for n in if_first + 1..=endif_last + 1 {
                        removed.insert(n);
                    }
                }
            }
            continue;
        }
        for (marker, (content_start, content_end, first)) in &group.dead_segments {
            if !first {
                if let Some(&(f, l)) = span_by_first.get(marker) {
                    delete_logical_span(&lines, f, l, &mut delete, &mut removed);
                } else if let Some(raw) = lines.get(marker - 1) {
                    delete.push((raw.start, raw.end));
                    removed.insert(*marker);
                }
            }
            if let (Some(start), Some(end)) = (content_start, content_end) {
                if let (Some(first_line), Some(last_line)) =
                    (lines.get(start - 1), lines.get(end - 1))
                {
                    delete.push((first_line.start, last_line.end));
                    for n in *start..=*end {
                        removed.insert(n);
                    }
                }
            }
        }
    }
    // Duplicate and dead tokens: remove the counted occurrences from
    // the end of the line, so a live first occurrence stays.
    let mut token_edits: HashMap<usize, Vec<(String, usize)>> = HashMap::new();
    for ((line, token), count) in &plan.duplicate_tokens {
        if !removed.contains(line) {
            token_edits
                .entry(*line)
                .or_default()
                .push((token.clone(), *count));
        }
    }
    for ((line, token), count) in &plan.dead_tokens {
        if !removed.contains(line) {
            token_edits
                .entry(*line)
                .or_default()
                .push((token.clone(), *count));
        }
    }
    for (line_no, edits) in &token_edits {
        let Some(&(first, last)) = span_by_first.get(line_no) else {
            continue;
        };
        let (effective, raw_offsets) = logical_effective(own_config, &lines, first, last);
        let (words, comment) = line_words(&effective);
        if words.len() <= 1 {
            // The keyword alone, or a comment-only line: nothing
            // to edit.
            continue;
        }
        // The word indices to remove, from the end of the line, so
        // a live first occurrence stays.
        let mut remove_idx: Vec<usize> = Vec::new();
        for (token, count) in edits {
            let mut to_remove = *count;
            for (idx, (s, e)) in words.iter().enumerate().rev() {
                if to_remove == 0 {
                    break;
                }
                if idx == 0 {
                    continue;
                }
                if effective[*s..*e] == *token {
                    remove_idx.push(idx);
                    to_remove -= 1;
                }
            }
        }
        if remove_idx.is_empty() {
            continue;
        }
        // The raw byte range of an effective byte range.
        let raw_range =
            |s: usize, e: usize| -> (usize, usize) { (raw_offsets[s], raw_offsets[e - 1] + 1) };
        if remove_idx.len() >= words.len() - 1 {
            // Every token goes: the line goes too, unless only a
            // comment is left — then the line becomes the comment
            // rather than deleting the author's note.
            match comment {
                Some(c) => {
                    let (comment_start, _) = raw_range(c, c + 1);
                    delete.push((lines[first].start, comment_start));
                }
                None => {
                    delete_logical_span(&lines, first, last, &mut delete, &mut removed);
                }
            }
            continue;
        }
        remove_idx.sort_unstable();
        for idx in remove_idx {
            let (s, e) = words[idx];
            let (word_start, word_end) = raw_range(s, e);
            if idx + 1 < words.len() {
                // A word follows: the token and the whitespace
                // after it go, so the surviving tokens keep their
                // spacing.
                let (next_start, _) = raw_range(words[idx + 1].0, words[idx + 1].0 + 1);
                delete.push((word_start, next_start));
            } else if let Some(c) = comment {
                // A comment follows: the token and the whitespace
                // before the comment go.
                let (comment_start, _) = raw_range(c, c + 1);
                delete.push((word_start, comment_start));
            } else {
                // The last word: the whitespace before it and the
                // token go.
                let (prev_start, prev_end) = words[idx - 1];
                let (_, prev_raw_end) = raw_range(prev_start, prev_end);
                delete.push((prev_raw_end, word_end));
            }
        }
    }
    // Merge overlapping delete ranges and splice them out.
    delete.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (s, e) in delete {
        if s >= e {
            continue;
        }
        if let Some(last) = merged.last_mut() {
            if s <= last.1 {
                last.1 = last.1.max(e);
                continue;
            }
        }
        merged.push((s, e));
    }
    let mut cleaned = String::with_capacity(own_config.len());
    let mut pos = 0;
    for (s, e) in &merged {
        cleaned.push_str(&own_config[pos..*s]);
        pos = *e;
    }
    cleaned.push_str(&own_config[pos..]);
    // A config without a trailing newline stays without one: when
    // the last line went, the previous line's terminator is a
    // separator, not a trailing newline.
    if !own_config.ends_with('\n') {
        if let Some(stripped) = cleaned
            .strip_suffix("\r\n")
            .or_else(|| cleaned.strip_suffix('\n'))
        {
            cleaned = stripped.to_string();
        }
    }
    // The lines removed: every physical line whose whole span is gone.
    let mut removed_lines: Vec<usize> = Vec::new();
    let mut range_idx = 0;
    for (idx, line) in lines.iter().enumerate() {
        while range_idx < merged.len() && merged[range_idx].1 <= line.start {
            range_idx += 1;
        }
        if range_idx < merged.len()
            && merged[range_idx].0 <= line.start
            && line.end <= merged[range_idx].1
        {
            removed_lines.push(idx + 1);
        }
    }
    (cleaned, removed_lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Credentials;
    use crate::debug::Debug;
    use crate::error::ErrorKind;
    use crate::obs::test_support::{basic_credentials, FakeHttp};

    fn macros(pairs: &[(&str, &str)]) -> MacroTable {
        let mut table = MacroTable::default();
        for (name, value) in pairs {
            table.defs.insert((*name).to_string(), (*value).to_string());
        }
        table
    }

    #[test]
    fn expand_macros_handles_the_common_forms() {
        let table = macros(&[("foo", "bar"), ("empty", "")]);
        assert_eq!(expand_macros("%{foo}", &table, 0).unwrap(), "bar");
        assert_eq!(expand_macros("%foo", &table, 0).unwrap(), "bar");
        assert_eq!(expand_macros("a%{foo}b", &table, 0).unwrap(), "abarb");
        assert_eq!(expand_macros("%{?foo}", &table, 0).unwrap(), "bar");
        assert_eq!(expand_macros("%{?missing}", &table, 0).unwrap(), "");
        assert_eq!(expand_macros("%{?missing:yes}", &table, 0).unwrap(), "");
        assert_eq!(expand_macros("%{?foo:yes}", &table, 0).unwrap(), "yes");
        assert_eq!(expand_macros("%{!?missing:yes}", &table, 0).unwrap(), "yes");
        assert_eq!(expand_macros("%{!?foo:yes}", &table, 0).unwrap(), "");
        assert_eq!(expand_macros("%{foo:-def}", &table, 0).unwrap(), "bar");
        assert_eq!(expand_macros("%{missing:-def}", &table, 0).unwrap(), "def");
        assert_eq!(expand_macros("%{empty:-def}", &table, 0).unwrap(), "def");
        assert_eq!(expand_macros("100%%", &table, 0).unwrap(), "100%");
        assert!(expand_macros("%{missing}", &table, 0).is_err());
        assert!(expand_macros("%missing", &table, 0).is_err());
        assert!(expand_macros("%{foo:+x}", &table, 0).is_err());
    }

    #[test]
    fn expand_macros_caps_expanded_length() {
        // One macro value may not expand past 1 MiB: two 600 KiB
        // values side by side are unevaluatable, one alone is not.
        let big = "x".repeat(600_000);
        let table = macros(&[("big", &big), ("two", "%{big}%{big}")]);
        assert_eq!(expand_macros("%{big}", &table, 0).unwrap().len(), 600_000);
        assert!(expand_macros("%{two}", &table, 0).is_err());
    }
    #[test]
    fn expand_macros_caps_expanded_bytes_per_evaluation() {
        // One 1 MiB value expanded once per call on one table: each call
        // is under the per-call length cap, so only the per-evaluation
        // byte budget can refuse the fifth call. One reference charges
        // 2 MiB (the recursive expansion plus the push of its result),
        // so four calls land exactly on the 8 MiB budget.
        let big = "x".repeat(1 << 20);
        let table = macros(&[("big", &big)]);
        assert_eq!(expand_macros("%{big}", &table, 0).unwrap().len(), 1 << 20);
        for _ in 0..3 {
            assert_eq!(expand_macros("%{big}", &table, 0).unwrap().len(), 1 << 20);
        }
        assert!(expand_macros("%{big}", &table, 0).is_err());
    }

    #[test]
    fn continued_marker_does_not_lend_its_continuation_to_the_next_segment() {
        // `%elif 0 == \` plus its continuation line is one logical
        // marker: the next segment's content starts after the
        // continuation, not after the marker's first line.
        let outcome = eval_own(
            "%if 0\nPrefer: a\n%elif 0 == \\\n  1\nPrefer: b\n%endif\n",
            "x86_64",
        );
        let dead = outcome.plan.dead_groups.get(&1).expect("group is dead");
        assert_eq!(dead.dead_segments.get(&3), Some(&(Some(5), Some(5), false)));
    }

    #[test]
    fn logical_effective_drops_a_trailing_backslash_on_an_unterminated_final_span() {
        // `logical_lines` drops the backslash of a final line ending
        // in `\` with no trailing newline; the effective text must
        // match, or token edits would see a stray `\` word.
        let text = "Prefer: foo \\";
        let lines = raw_lines(text);
        let spans = logical_spans(&lines, text);
        assert_eq!(spans, vec![(0, 0)]);
        let (effective, _) = logical_effective(text, &lines, 0, 0);
        assert_eq!(effective, "Prefer: foo ");
        assert_eq!(logical_lines(text)[0].text, "Prefer: foo ");
    }

    #[test]
    fn dead_branch_ids_distinguish_same_project_sections() {
        // Two sections of one project can hold a branch at the same
        // offset from their marker; the section ordinal keeps their
        // identities apart.
        let buildconfig = "### from home:test\n%if 0\nPrefer: a\n%endif\n### from home:test\n%if 0\nPrefer: b\n%endif\n";
        let outcome = evaluate_buildconfig(
            buildconfig,
            "x86_64",
            "standard",
            "home:test",
            &HashMap::new(),
        );
        assert_eq!(outcome.plan.dead_branch_ids.len(), 2);
    }

    #[test]
    fn dead_branch_ids_are_not_emitted_without_markers() {
        // Without a marker the offset is an absolute buildconfig
        // line, so different branches in different repositories
        // could share one id: no cross-repository identity is
        // emitted at all.
        let buildconfig = "%if 0\nPrefer: a\n%endif\n";
        let outcome = evaluate_buildconfig(
            buildconfig,
            "x86_64",
            "standard",
            "home:test",
            &HashMap::new(),
        );
        assert!(outcome.plan.dead_branch_ids.is_empty());
    }

    #[test]
    fn eval_condition_handles_comparisons_and_logic() {
        let table = macros(&[("arch", "x86_64")]);
        assert!(eval_condition("1", &table).unwrap());
        assert!(!eval_condition("0", &table).unwrap());
        assert!(eval_condition("1 == 1", &table).unwrap());
        assert!(!eval_condition("1 == 2", &table).unwrap());
        assert!(eval_condition("1 != 2", &table).unwrap());
        assert!(eval_condition("5 > 3", &table).unwrap());
        assert!(eval_condition("3 <= 3", &table).unwrap());
        assert!(!eval_condition("1 && 0", &table).unwrap());
        assert!(eval_condition("1 || 0", &table).unwrap());
        assert!(eval_condition("!0", &table).unwrap());
        assert!(eval_condition("%{arch} == \"x86_64\"", &table).unwrap());
        assert!(!eval_condition("%{arch} == \"aarch64\"", &table).unwrap());
        assert!(eval_condition("(%{arch} == \"x86_64\") && 1", &table).unwrap());
        // A relational comparison of non-numeric strings is
        // unevaluatable, never guessed.
        assert!(eval_condition("\"a\" > \"b\"", &table).is_err());
        assert!(eval_condition("15.6 > 15", &table).is_err());
        assert!(eval_condition("%{missing} == 1", &table).is_err());
    }

    /// Evaluate an own config as if it were the whole buildconfig of
    /// one repository.
    fn eval_own(own_config: &str, arch: &str) -> EvalOutcome {
        let buildconfig = format!(
            "### from home:test\n%define _repository standard\n%define _project home:test\n%define _is_this_project 1\n{own_config}"
        );
        let line_map = map_buildconfig_to_own(&buildconfig, own_config, "home:test")
            .expect("test buildconfig maps");
        evaluate_buildconfig(&buildconfig, arch, "standard", "home:test", &line_map)
    }

    fn has_finding(outcome: &EvalOutcome, kind: PrjconfFindingKind) -> bool {
        outcome.findings.iter().any(|f| f.kind == kind)
    }

    fn finding_lines(outcome: &EvalOutcome, kind: PrjconfFindingKind) -> Vec<usize> {
        outcome
            .findings
            .iter()
            .filter(|f| f.kind == kind)
            .filter_map(|f| f.line)
            .collect()
    }

    #[test]
    fn dead_removal_of_a_never_added_token() {
        let outcome = eval_own("Prefer: foo\nPrefer: !bar\n", "x86_64");
        assert!(has_finding(&outcome, PrjconfFindingKind::DeadRemoval));
        assert_eq!(
            finding_lines(&outcome, PrjconfFindingKind::DeadRemoval),
            vec![2]
        );
        assert_eq!(
            outcome.plan.dead_tokens.get(&(2, "!bar".to_string())),
            Some(&1)
        );
        assert!(!outcome.plan.dead_lines.contains(&2));
        let (cleaned, removed) = apply_plan("Prefer: foo\nPrefer: !bar\n", &outcome.plan);
        assert_eq!(cleaned, "Prefer: foo\n");
        assert_eq!(removed, vec![2]);
    }

    #[test]
    fn live_removal_of_an_added_token_is_not_dead() {
        let outcome = eval_own("Prefer: foo\nPrefer: !foo\n", "x86_64");
        assert!(!has_finding(&outcome, PrjconfFindingKind::DeadRemoval));
        assert!(outcome.plan.dead_lines.is_empty());
        let prefer = outcome.state.lists.get("prefer").unwrap();
        assert!(prefer.is_empty());
    }

    #[test]
    fn prefer_split_across_lines_accumulates() {
        // The correction to the issue: a multi-line `Prefer` is one
        // list, so a removal of a token added on an earlier line is
        // live, not dead.
        let outcome = eval_own("Prefer: foo\nPrefer: bar\nPrefer: !foo\n", "x86_64");
        assert!(!has_finding(&outcome, PrjconfFindingKind::DeadRemoval));
        let prefer = outcome.state.lists.get("prefer").unwrap();
        assert_eq!(
            prefer.iter().map(|t| t.token.as_str()).collect::<Vec<_>>(),
            vec!["bar"]
        );
    }

    #[test]
    fn dead_clear_of_an_empty_list() {
        let outcome = eval_own("Prefer: !*\n", "x86_64");
        assert!(has_finding(&outcome, PrjconfFindingKind::DeadClear));
        assert_eq!(
            outcome.plan.dead_tokens.get(&(1, "!*".to_string())),
            Some(&1)
        );
        assert!(!outcome.plan.dead_lines.contains(&1));
        let (cleaned, removed) = apply_plan("Prefer: !*\n", &outcome.plan);
        assert_eq!(cleaned, "");
        assert_eq!(removed, vec![1]);
        let outcome = eval_own("Prefer: foo\nPrefer: !*\n", "x86_64");
        assert!(!has_finding(&outcome, PrjconfFindingKind::DeadClear));
    }

    #[test]
    fn dead_removal_on_a_mixed_line_keeps_the_live_token() {
        let outcome = eval_own("Prefer: !never-added live-pkg\n", "x86_64");
        assert!(has_finding(&outcome, PrjconfFindingKind::DeadRemoval));
        assert_eq!(
            outcome
                .plan
                .dead_tokens
                .get(&(1, "!never-added".to_string())),
            Some(&1)
        );
        let (cleaned, removed) = apply_plan("Prefer: !never-added live-pkg\n", &outcome.plan);
        assert_eq!(cleaned, "Prefer: live-pkg\n");
        assert!(removed.is_empty());
    }

    #[test]
    fn dead_clear_on_a_mixed_line_keeps_the_live_token() {
        let outcome = eval_own("Prefer: !* live-pkg\n", "x86_64");
        assert!(has_finding(&outcome, PrjconfFindingKind::DeadClear));
        assert_eq!(
            outcome.plan.dead_tokens.get(&(1, "!*".to_string())),
            Some(&1)
        );
        let (cleaned, removed) = apply_plan("Prefer: !* live-pkg\n", &outcome.plan);
        assert_eq!(cleaned, "Prefer: live-pkg\n");
        assert!(removed.is_empty());
    }

    #[test]
    fn duplicate_token_is_flagged_and_counted() {
        let outcome = eval_own("Prefer: foo foo foo\n", "x86_64");
        assert!(has_finding(&outcome, PrjconfFindingKind::DuplicateToken));
        assert_eq!(
            outcome.plan.duplicate_tokens.get(&(1, "foo".to_string())),
            Some(&2)
        );
    }

    #[test]
    fn overridden_substitute_marks_the_earlier_line_dead() {
        let outcome = eval_own("Substitute: foo bar\nSubstitute: foo baz\n", "x86_64");
        assert!(has_finding(
            &outcome,
            PrjconfFindingKind::OverriddenSubstitute
        ));
        assert_eq!(
            finding_lines(&outcome, PrjconfFindingKind::OverriddenSubstitute),
            vec![1]
        );
        assert!(outcome.plan.dead_lines.contains(&1));
        let entry = outcome.state.substitute.get("foo").unwrap();
        assert_eq!(entry.values, vec!["baz"]);
    }

    #[test]
    fn dead_branch_is_flagged_and_whole_group_dead() {
        let outcome = eval_own("%if 0\nPrefer: foo\n%endif\n", "x86_64");
        assert!(has_finding(&outcome, PrjconfFindingKind::DeadBranch));
        assert_eq!(
            finding_lines(&outcome, PrjconfFindingKind::DeadBranch),
            vec![1]
        );
        let group = outcome.plan.dead_groups.get(&1).unwrap();
        assert!(group.all_dead);
        let prefer = outcome.state.lists.get("prefer").unwrap();
        assert!(prefer.is_empty());
    }

    #[test]
    fn live_branch_applies_its_lines() {
        let outcome = eval_own("%if 1\nPrefer: foo\n%else\nPrefer: bar\n%endif\n", "x86_64");
        // The `%else` segment is dead (the `%if` branch was taken);
        // the `%if` segment is live.
        assert!(has_finding(&outcome, PrjconfFindingKind::DeadBranch));
        assert_eq!(
            finding_lines(&outcome, PrjconfFindingKind::DeadBranch),
            vec![3]
        );
        let prefer = outcome.state.lists.get("prefer").unwrap();
        assert_eq!(
            prefer.iter().map(|t| t.token.as_str()).collect::<Vec<_>>(),
            vec!["foo"]
        );
        // The %else segment is dead; only its content goes, not the
        // marker.
        let group = outcome.plan.dead_groups.get(&1).unwrap();
        assert!(!group.all_dead);
        assert!(group.dead_segments.contains_key(&3));
    }

    #[test]
    fn ifarch_evaluates_against_the_arch() {
        let outcome = eval_own("%ifarch x86_64\nPrefer: foo\n%endif\n", "x86_64");
        assert!(!has_finding(&outcome, PrjconfFindingKind::DeadBranch));
        let outcome = eval_own("%ifarch aarch64\nPrefer: foo\n%endif\n", "x86_64");
        assert!(has_finding(&outcome, PrjconfFindingKind::DeadBranch));
        let outcome = eval_own("%ifnarch x86_64\nPrefer: foo\n%endif\n", "x86_64");
        assert!(has_finding(&outcome, PrjconfFindingKind::DeadBranch));
    }

    #[test]
    fn unevaluatable_condition_poisons_and_skips() {
        let outcome = eval_own("%if %{missing} == 1\nPrefer: foo\n%endif\n", "x86_64");
        assert!(!has_finding(&outcome, PrjconfFindingKind::DeadBranch));
        assert!(!outcome.skipped.is_empty());
        // The line inside the uncertain branch never applied.
        let prefer = outcome.state.lists.get("prefer").unwrap();
        assert!(prefer.is_empty());
        // The list is poisoned: a later removal cannot be proven dead.
        let outcome = eval_own(
            "%if %{missing} == 1\nPrefer: foo\n%endif\nPrefer: !foo\n",
            "x86_64",
        );
        assert!(!has_finding(&outcome, PrjconfFindingKind::DeadRemoval));
    }

    #[test]
    fn unknown_keyword_is_a_warning() {
        let outcome = eval_own("Frobnicate: foo\n", "x86_64");
        assert!(has_finding(&outcome, PrjconfFindingKind::UnknownKeyword));
        assert_eq!(
            finding_lines(&outcome, PrjconfFindingKind::UnknownKeyword),
            vec![1]
        );
    }

    #[test]
    fn fromscratch_first_is_fine_later_is_a_parse_error() {
        let outcome = eval_own("FromScratch: 1\nPrefer: foo\n", "x86_64");
        assert!(!has_finding(&outcome, PrjconfFindingKind::ParseError));
        let outcome = eval_own("Prefer: foo\nFromScratch: 1\n", "x86_64");
        assert!(has_finding(&outcome, PrjconfFindingKind::ParseError));
        assert_eq!(
            outcome
                .findings
                .iter()
                .find(|f| f.kind == PrjconfFindingKind::ParseError)
                .unwrap()
                .severity,
            PrjconfSeverity::Error
        );
    }

    #[test]
    fn unclosed_if_is_an_unclosed_if_error() {
        let outcome = eval_own("%if 1\nPrefer: foo\n", "x86_64");
        assert!(has_finding(&outcome, PrjconfFindingKind::UnclosedIf));
        assert_eq!(
            outcome
                .findings
                .iter()
                .find(|f| f.kind == PrjconfFindingKind::UnclosedIf)
                .unwrap()
                .severity,
            PrjconfSeverity::Error
        );
    }

    #[test]
    fn stray_endif_is_a_parse_error() {
        let outcome = eval_own("Prefer: foo\n%endif\n", "x86_64");
        assert!(has_finding(&outcome, PrjconfFindingKind::ParseError));
    }

    #[test]
    fn macros_section_defines_nothing_for_the_solver() {
        // The scheduler strips `Macros:` sections, so a macro defined
        // inside one is not available to a later directive. The
        // buildconfig below is the server's layout for this config: a
        // flushed `Macros:` block precedes the config's own lines
        // (the config holds a `:macros` line), and the first
        // `macros:` section stays in place.
        let own = "Macros:\n%define foo bar\n:macros\nPrefer: %{foo}\n";
        let buildconfig = "%define _project home:test\n### from home:test\n%define _repository standard\n%define _project home:test\n%define _is_this_project 1\n#!!line home:test:0\n\nMacros:\n%vendor obs://build.opensuse.org/home:test\n%_project home:test\n%distribution home:test\n%_project home:test\n### from home:test\n%_repository standard\n:Macros\n\nMacros:\n%define foo bar\n:macros\nPrefer: %{foo}\n";
        let line_map =
            map_buildconfig_to_own(buildconfig, own, "home:test").expect("test buildconfig maps");
        let outcome =
            evaluate_buildconfig(buildconfig, "x86_64", "standard", "home:test", &line_map);
        assert!(!outcome.skipped.is_empty());
        let prefer = outcome.state.lists.get("prefer").unwrap();
        assert!(prefer.is_empty());
    }

    #[test]
    fn define_outside_a_macros_section_applies() {
        let outcome = eval_own("%define foo bar\nPrefer: %{foo}\n", "x86_64");
        let prefer = outcome.state.lists.get("prefer").unwrap();
        assert_eq!(
            prefer.iter().map(|t| t.token.as_str()).collect::<Vec<_>>(),
            vec!["bar"]
        );
    }

    #[test]
    fn map_buildconfig_to_own_handles_a_macros_section_in_place() {
        // The server's layout for a config holding a `:macros` line:
        // a flushed `Macros:` block precedes the config's own lines,
        // and the first `macros:` section stays in place.
        let own = "Prefer: foo\nMacros:\n%define bar baz\n:macros\nSupport: foo\n";
        let buildconfig = "%define _project home:test\n### from home:test\n%define _repository standard\n%define _is_this_project 1\n#!!line home:test:0\n\nMacros:\n%vendor obs://build.opensuse.org/home:test\n%_project home:test\n%distribution home:test\n%_project home:test\n### from home:test\n%_repository standard\n:Macros\n\nPrefer: foo\nMacros:\n%define bar baz\n:macros\nSupport: foo\n";
        let map = map_buildconfig_to_own(buildconfig, own, "home:test").expect("maps");
        assert_eq!(map.get(&16), Some(&1));
        assert_eq!(map.get(&17), Some(&2));
        assert_eq!(map.get(&20), Some(&5));
    }

    #[test]
    fn map_buildconfig_to_own_handles_a_relocated_macros_section() {
        // The server's layout for a config holding an unclosed
        // `macros:` section: the section is relocated to the trailing
        // `Macros:` section.
        let own = "Prefer: foo\nMacros:\n%define bar baz\n";
        let buildconfig = "%define _project home:test\n### from home:test\n%define _repository standard\n%define _is_this_project 1\n#!!line home:test:0\nPrefer: foo\n\nMacros:\n%vendor obs://build.opensuse.org/home:test\n%_project home:test\n### from home:test\n%_repository standard\nmacros:\n%define bar baz\n";
        let map = map_buildconfig_to_own(buildconfig, own, "home:test").expect("maps");
        assert_eq!(map.get(&6), Some(&1));
    }

    #[test]
    fn map_buildconfig_to_own_refuses_a_mismatch() {
        let own = "Prefer: foo\n";
        let buildconfig = "### from home:test\nPrefer: bar\n";
        assert!(map_buildconfig_to_own(buildconfig, own, "home:test").is_none());
    }

    #[test]
    fn intersect_plans_keeps_only_what_is_dead_everywhere() {
        let mut a = EvalPlan::default();
        a.dead_lines.insert(2);
        a.dead_lines.insert(3);
        a.duplicate_tokens.insert((4, "foo".to_string()), 2);
        a.dead_tokens.insert((5, "!bar".to_string()), 2);
        a.dead_tokens.insert((6, "!baz".to_string()), 1);
        let mut b = EvalPlan::default();
        b.dead_lines.insert(3);
        b.duplicate_tokens.insert((4, "foo".to_string()), 1);
        b.dead_tokens.insert((5, "!bar".to_string()), 1);
        let plan = intersect_plans(vec![a, b]);
        assert_eq!(plan.dead_lines, HashSet::from([3]));
        assert_eq!(plan.duplicate_tokens.get(&(4, "foo".to_string())), Some(&1));
        assert_eq!(plan.dead_tokens.get(&(5, "!bar".to_string())), Some(&1));
        assert_eq!(plan.dead_tokens.get(&(6, "!baz".to_string())), None);
    }

    #[test]
    fn apply_plan_removes_dead_lines_and_duplicate_tokens() {
        let own = "Prefer: foo bar\nPrefer: !baz\nPrefer: foo foo\n";
        let mut plan = EvalPlan::default();
        plan.dead_lines.insert(2);
        plan.duplicate_tokens.insert((3, "foo".to_string()), 1);
        let (cleaned, removed) = apply_plan(own, &plan);
        assert_eq!(cleaned, "Prefer: foo bar\nPrefer: foo\n");
        assert_eq!(removed, vec![2]);
    }

    #[test]
    fn apply_plan_removes_dead_tokens_and_keeps_live_tokens() {
        let own = "Prefer: !bar live-pkg\nPrefer: !baz\n";
        let mut plan = EvalPlan::default();
        plan.dead_tokens.insert((1, "!bar".to_string()), 1);
        plan.dead_tokens.insert((2, "!baz".to_string()), 1);
        let (cleaned, removed) = apply_plan(own, &plan);
        assert_eq!(cleaned, "Prefer: live-pkg\n");
        assert_eq!(removed, vec![2]);
    }

    #[test]
    fn apply_plan_removes_a_wholly_dead_group() {
        let own = "%if 0\nPrefer: foo\n%endif\nPrefer: bar\n";
        let mut plan = EvalPlan::default();
        let group = GroupDead {
            all_dead: true,
            endif_own_line: Some(3),
            ..Default::default()
        };
        plan.dead_groups.insert(1, group);
        let (cleaned, removed) = apply_plan(own, &plan);
        assert_eq!(cleaned, "Prefer: bar\n");
        assert_eq!(removed, vec![1, 2, 3]);
    }

    #[test]
    fn apply_plan_keeps_the_if_marker_of_a_dead_first_segment() {
        let own = "%if 0\nPrefer: foo\n%elif 1\nPrefer: bar\n%endif\n";
        let mut plan = EvalPlan::default();
        let mut group = GroupDead {
            all_dead: false,
            endif_own_line: Some(5),
            ..Default::default()
        };
        group.dead_segments.insert(1, (Some(2), Some(2), true));
        plan.dead_groups.insert(1, group);
        let (cleaned, removed) = apply_plan(own, &plan);
        assert_eq!(cleaned, "%if 0\n%elif 1\nPrefer: bar\n%endif\n");
        assert_eq!(removed, vec![2]);
    }

    #[test]
    fn apply_plan_drops_a_line_when_all_its_tokens_are_duplicates() {
        let own = "Prefer: foo\nPrefer: foo foo\n";
        let mut plan = EvalPlan::default();
        plan.duplicate_tokens.insert((2, "foo".to_string()), 2);
        let (cleaned, removed) = apply_plan(own, &plan);
        assert_eq!(cleaned, "Prefer: foo\n");
        assert_eq!(removed, vec![2]);
    }

    /// Evaluate a two-section buildconfig: an inherited config from
    /// openSUSE:Factory, then the own config.
    fn eval_two_section(inherited: &str, own: &str, arch: &str) -> EvalOutcome {
        let buildconfig = format!(
            "### from openSUSE:Factory\n%define _repository standard\n%define _project openSUSE:Factory\n{inherited}### from home:test\n%define _repository standard\n%define _project home:test\n%define _is_this_project 1\n{own}"
        );
        let line_map =
            map_buildconfig_to_own(&buildconfig, own, "home:test").expect("test buildconfig maps");
        evaluate_buildconfig(&buildconfig, arch, "standard", "home:test", &line_map)
    }

    #[test]
    fn shadowed_avoid_loses_to_an_inherited_support() {
        let mut outcome = eval_two_section("Support: foo\n", "Prefer: -foo\n", "x86_64");
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/openSUSE:Factory",
                "<directory><entry name=\"foo\"/></directory>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:test",
                "<directory></directory>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let path_projects: HashSet<String> =
            ["openSUSE:Factory".to_string(), "home:test".to_string()]
                .into_iter()
                .collect();
        let mut package_cache = HashMap::new();
        let mut owner_cache = HashMap::new();
        client.prjconf_resolve_and_shadow(
            &mut outcome,
            "home:test",
            "standard",
            &path_projects,
            &mut package_cache,
            &mut owner_cache,
            &mut OwnerLookupBudget::new(),
        );
        assert!(has_finding(&outcome, PrjconfFindingKind::ShadowedAvoid));
        assert_eq!(
            finding_lines(&outcome, PrjconfFindingKind::ShadowedAvoid),
            vec![1]
        );
        // The avoided package resolves, so no unresolvable finding.
        assert!(!has_finding(
            &outcome,
            PrjconfFindingKind::UnresolvableTarget
        ));
    }

    #[test]
    fn unresolvable_target_is_flagged_when_no_package_or_binary() {
        let mut outcome = eval_two_section("", "Prefer: nosuchpkg\n", "x86_64");
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/openSUSE:Factory",
                "<directory><entry name=\"foo\"/></directory>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:test",
                "<directory></directory>",
            )
            .with_get(
                "https://api.opensuse.org/search/owner?binary=nosuchpkg",
                "<collection matches=\"0\"></collection>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let path_projects: HashSet<String> =
            ["openSUSE:Factory".to_string(), "home:test".to_string()]
                .into_iter()
                .collect();
        let mut package_cache = HashMap::new();
        let mut owner_cache = HashMap::new();
        client.prjconf_resolve_and_shadow(
            &mut outcome,
            "home:test",
            "standard",
            &path_projects,
            &mut package_cache,
            &mut owner_cache,
            &mut OwnerLookupBudget::new(),
        );
        assert!(has_finding(
            &outcome,
            PrjconfFindingKind::UnresolvableTarget
        ));
        assert_eq!(
            finding_lines(&outcome, PrjconfFindingKind::UnresolvableTarget),
            vec![1]
        );
    }

    #[test]
    fn a_binary_owned_by_a_path_project_resolves() {
        let mut outcome = eval_two_section("", "Prefer: somebinary\n", "x86_64");
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/openSUSE:Factory",
                "<directory><entry name=\"foo\"/></directory>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:test",
                "<directory></directory>",
            )
            .with_get(
                "https://api.opensuse.org/search/owner?binary=somebinary",
                "<collection matches=\"1\"><owner rootproject=\"openSUSE:Factory\" project=\"openSUSE:Factory\" package=\"somepkg\"><person name=\"admin\" role=\"maintainer\"/></owner></collection>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let path_projects: HashSet<String> =
            ["openSUSE:Factory".to_string(), "home:test".to_string()]
                .into_iter()
                .collect();
        let mut package_cache = HashMap::new();
        let mut owner_cache = HashMap::new();
        client.prjconf_resolve_and_shadow(
            &mut outcome,
            "home:test",
            "standard",
            &path_projects,
            &mut package_cache,
            &mut owner_cache,
            &mut OwnerLookupBudget::new(),
        );
        assert!(!has_finding(
            &outcome,
            PrjconfFindingKind::UnresolvableTarget
        ));
    }

    #[test]
    fn validate_full_flow_reports_a_dead_removal() {
        let own = "Prefer: foo\nPrefer: !bar\n";
        let buildconfig = format!(
            "### from home:test\n%define _repository standard\n%define _project home:test\n%define _is_this_project 1\n{own}"
        );
        let meta = "<project name=\"home:test\"><repository name=\"standard\"><arch>x86_64</arch><path project=\"home:test\" repository=\"standard\"/></repository></project>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:test/_meta",
                meta,
            )
            .with_get("https://api.opensuse.org/source/home:test/_config", own)
            .with_get(
                "https://api.opensuse.org/build/home:test/standard/_buildconfig",
                &buildconfig,
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:test",
                "<directory><entry name=\"foo\"/></directory>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let validation = client.validate_project_config("home:test").unwrap();
        assert_eq!(validation.project, "home:test");
        assert_eq!(validation.findings.len(), 1);
        assert_eq!(validation.findings[0].kind, PrjconfFindingKind::DeadRemoval);
        assert_eq!(validation.findings[0].line, Some(2));
    }

    #[test]
    fn validate_full_flow_own_package_resolves_without_a_path() {
        // The live prjconf fixture carries no path: the API rejects
        // a repository pathing to itself. The own package still
        // resolves, because the validator counts the own project as
        // on the path.
        let own = "Prefer: foo\n";
        let buildconfig = format!(
            "### from home:test\n%define _repository standard\n%define _project home:test\n%define _is_this_project 1\n{own}"
        );
        let meta = "<project name=\"home:test\"><repository name=\"standard\"><arch>x86_64</arch></repository></project>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:test/_meta",
                meta,
            )
            .with_get("https://api.opensuse.org/source/home:test/_config", own)
            .with_get(
                "https://api.opensuse.org/build/home:test/standard/_buildconfig",
                &buildconfig,
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:test",
                "<directory><entry name=\"foo\"/></directory>",
            )
            // The owner lookup only runs when the package list does not resolve
            // the name: it answers "owned elsewhere", so a missing own-project
            // chain still fails instead of leaving the name alone.
            .with_get(
                "https://api.opensuse.org/search/owner?binary=foo",
                "<collection matches=\"1\"><owner rootproject=\"openSUSE:Factory\" project=\"openSUSE:Factory\" package=\"somepkg\"><person name=\"admin\" role=\"maintainer\"/></owner></collection>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let validation = client.validate_project_config("home:test").unwrap();
        assert!(
            !validation
                .findings
                .iter()
                .any(|finding| finding.kind == PrjconfFindingKind::UnresolvableTarget),
            "the own package must resolve with no path: {:?}",
            validation.findings
        );
    }

    fn meta_with_repositories(count: usize, with_arch: bool) -> String {
        let mut meta = String::from("<project name=\"home:test\">");
        for i in 0..count {
            meta.push_str(&format!("<repository name=\"repo{i}\">"));
            if with_arch {
                meta.push_str("<arch>x86_64</arch>");
            }
            meta.push_str(&format!(
                "<path project=\"home:test\" repository=\"repo{i}\"/>"
            ));
            meta.push_str("</repository>");
        }
        meta.push_str("</project>");
        meta
    }

    #[test]
    fn validate_fails_closed_without_credentials_and_issues_no_request() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/home:test/_meta",
            &meta_with_repositories(1, true),
        );
        let client = ObsClient::with_credentials(&http, Credentials::None).unwrap();
        let err = client
            .validate_project_config("home:test")
            .expect_err("validate without credentials must fail");
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("without OBS credentials"));
        assert!(
            http.get_calls().is_empty(),
            "no anonymous request is attempted: {:?}",
            http.get_calls()
        );
    }

    #[test]
    fn clean_fails_closed_without_credentials_and_issues_no_request() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/home:test/_meta",
            &meta_with_repositories(1, true),
        );
        let client = ObsClient::with_credentials(&http, Credentials::None).unwrap();
        let err = client
            .clean_project_config("home:test", &DryRun::new(true), false)
            .expect_err("clean without credentials must fail");
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("without OBS credentials"));
        assert!(
            http.get_calls().is_empty(),
            "no anonymous request is attempted: {:?}",
            http.get_calls()
        );
    }

    #[test]
    fn validate_refuses_a_project_above_the_repository_bound() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/home:test/_meta",
            &meta_with_repositories(33, false),
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .validate_project_config("home:test")
            .expect_err("33 repositories must refuse");
        assert_eq!(err.kind(), ErrorKind::TooManyRepositories);
        assert!(err.context.contains("33"));
        assert!(err.context.contains("32"));
        // Only the meta read ran: no config fetch, no buildconfig fetch.
        let calls = http.get_calls();
        let urls: Vec<&str> = calls.iter().map(|(url, _)| url.as_str()).collect();
        assert_eq!(
            urls,
            ["https://api.opensuse.org/public/source/home:test/_meta"]
        );
    }

    #[test]
    fn validate_accepts_a_project_at_the_repository_bound() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:test/_meta",
                &meta_with_repositories(32, false),
            )
            .with_get("https://api.opensuse.org/source/home:test/_config", "");
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let validation = client
            .validate_project_config("home:test")
            .expect("32 repositories must validate");
        assert_eq!(validation.findings.len(), 0);
        assert_eq!(validation.skipped.len(), 32);
        assert!(
            !http
                .get_calls()
                .iter()
                .any(|(url, _)| url.contains("_buildconfig")),
            "no buildconfig fetch for archless repositories"
        );
    }

    #[test]
    fn validate_reports_progress_per_repository_on_the_debug_channel() {
        let own = "Prefer: foo\nPrefer: !bar\n";
        let meta = "<project name=\"home:test\">\
            <repository name=\"standard\"><arch>x86_64</arch><path project=\"home:test\" repository=\"standard\"/></repository>\
            <repository name=\"other\"><arch>x86_64</arch><path project=\"home:test\" repository=\"other\"/></repository>\
            </project>";
        let buildconfig = |repo: &str| {
            format!(
                "### from home:test\n%define _repository {repo}\n%define _project home:test\n%define _is_this_project 1\n{own}"
            )
        };
        let (debug, lines) = Debug::captured(true, false);
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:test/_meta",
                meta,
            )
            .with_get("https://api.opensuse.org/source/home:test/_config", own)
            .with_get(
                "https://api.opensuse.org/build/home:test/standard/_buildconfig",
                &buildconfig("standard"),
            )
            .with_get(
                "https://api.opensuse.org/build/home:test/other/_buildconfig",
                &buildconfig("other"),
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:test",
                "<directory><entry name=\"foo\"/></directory>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials())
            .unwrap()
            .with_debug(debug);
        client
            .validate_project_config("home:test")
            .expect("validate runs");
        let got: Vec<String> = lines.lock().unwrap().clone();
        assert_eq!(
            got,
            [
                "prjconf validate: home:test: 2 repositories, 2 repository-architecture evaluations",
                "prjconf validate: home:test: repository 1/2 (standard)",
                "prjconf validate: home:test: repository 2/2 (other)",
            ]
        );
    }

    #[test]
    fn clean_dry_run_writes_nothing_and_live_writes_the_cleaned_config() {
        let own = "Prefer: foo\nPrefer: !bar\n";
        let buildconfig = format!(
            "### from home:test\n%define _repository standard\n%define _project home:test\n%define _is_this_project 1\n{own}"
        );
        let meta = "<project name=\"home:test\"><repository name=\"standard\"><arch>x86_64</arch><path project=\"home:test\" repository=\"standard\"/></repository></project>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:test/_meta",
                meta,
            )
            .with_get("https://api.opensuse.org/source/home:test/_config", own)
            .with_get(
                "https://api.opensuse.org/build/home:test/standard/_buildconfig",
                &buildconfig,
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:test",
                "<directory><entry name=\"foo\"/></directory>",
            )
            .with_put(
                "https://api.opensuse.org/source/home:test/_config",
                "<status code=\"ok\"/>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(true);
        let result = client
            .clean_project_config("home:test", &dry, true)
            .unwrap();
        assert!(!result.applied);
        assert_eq!(result.removed_lines, vec![2]);
        assert_eq!(result.cleaned, "Prefer: foo\n");
        assert!(http.put_calls().is_empty());
        let live = DryRun::new(false);
        let result = client
            .clean_project_config("home:test", &live, true)
            .unwrap();
        assert!(result.applied);
        assert_eq!(http.put_calls().len(), 1);
        assert_eq!(http.put_calls()[0].1, "Prefer: foo\n");
    }

    #[test]
    fn clean_refuses_when_the_config_has_a_parse_error() {
        let own = "Prefer: foo\nFromScratch: 1\n";
        let buildconfig = format!(
            "### from home:test\n%define _repository standard\n%define _project home:test\n%define _is_this_project 1\n{own}"
        );
        let meta = "<project name=\"home:test\"><repository name=\"standard\"><arch>x86_64</arch><path project=\"home:test\" repository=\"standard\"/></repository></project>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:test/_meta",
                meta,
            )
            .with_get("https://api.opensuse.org/source/home:test/_config", own)
            .with_get(
                "https://api.opensuse.org/build/home:test/standard/_buildconfig",
                &buildconfig,
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:test",
                "<directory><entry name=\"foo\"/></directory>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(true);
        let err = client
            .clean_project_config("home:test", &dry, false)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ValidationFailed);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn clean_with_failed_mapping_and_empty_own_config_is_a_no_op() {
        // The buildconfig's own section holds a line the empty own
        // config cannot map to, so the mapping fails — but there is
        // nothing to clean, so the clean succeeds as a no-op
        // instead of refusing.
        let own = "\n";
        let buildconfig = "### from home:test\n%define _repository standard\n%define _project home:test\n%define _is_this_project 1\n%define foo bar\n";
        let meta = "<project name=\"home:test\"><repository name=\"standard\"><arch>x86_64</arch><path project=\"home:test\" repository=\"standard\"/></repository></project>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:test/_meta",
                meta,
            )
            .with_get("https://api.opensuse.org/source/home:test/_config", own)
            .with_get(
                "https://api.opensuse.org/build/home:test/standard/_buildconfig",
                buildconfig,
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:test",
                "<directory></directory>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(true);
        let result = client
            .clean_project_config("home:test", &dry, false)
            .unwrap();
        assert!(!result.applied);
        assert!(result.removed_lines.is_empty());
        assert_eq!(result.cleaned, "\n");
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn clean_with_a_whitespace_only_own_config_is_a_no_op_when_the_mapping_succeeds() {
        // The buildconfig is just a newline, which maps onto the
        // whitespace-only own config — so the mapping succeeds,
        // and the clean is still a no-op returning the config
        // verbatim.
        let own = "\n";
        let buildconfig = "\n";
        let meta = "<project name=\"home:test\"><repository name=\"standard\"><arch>x86_64</arch><path project=\"home:test\" repository=\"standard\"/></repository></project>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:test/_meta",
                meta,
            )
            .with_get("https://api.opensuse.org/source/home:test/_config", own)
            .with_get(
                "https://api.opensuse.org/build/home:test/standard/_buildconfig",
                buildconfig,
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:test",
                "<directory></directory>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(true);
        let result = client
            .clean_project_config("home:test", &dry, false)
            .unwrap();
        assert!(!result.applied);
        assert!(result.removed_lines.is_empty());
        assert_eq!(result.cleaned, "\n");
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn uncertain_else_does_not_prove_a_duplicate() {
        // The `%else` runs only when the unevaluatable `%if` is false,
        // so the final `Prefer: baz` is not provably a duplicate.
        let outcome = eval_own(
            "%if %{missing} == 1\n%else\nPrefer: baz\n%endif\nPrefer: baz\n",
            "x86_64",
        );
        assert!(!has_finding(&outcome, PrjconfFindingKind::DuplicateToken));
        assert!(!outcome.plan.dead_lines.contains(&5));
    }

    #[test]
    fn uncertain_elif_does_not_prove_a_duplicate() {
        let outcome = eval_own(
            "%if %{missing} == 1\n%elif 1\nPrefer: baz\n%endif\nPrefer: baz\n",
            "x86_64",
        );
        assert!(!has_finding(&outcome, PrjconfFindingKind::DuplicateToken));
        assert!(!outcome.plan.dead_lines.contains(&5));
    }

    #[test]
    fn nested_if_under_an_uncertain_parent_is_not_dead() {
        // The inner `%if` is live whenever the outer condition is
        // true, so its lines are not dead.
        let outcome = eval_own(
            "%if %{missing} == 1\n%if 1\nPrefer: foo\n%endif\n%endif\n",
            "x86_64",
        );
        assert!(!outcome.plan.dead_lines.contains(&3));
        assert!(!has_finding(&outcome, PrjconfFindingKind::DeadBranch));
        assert!(outcome.plan.dead_groups.is_empty());
    }

    /// The tokens in the `prefer` list of an evaluation outcome.
    fn prefer_tokens(outcome: &EvalOutcome) -> Vec<String> {
        outcome
            .state
            .lists
            .get("prefer")
            .map(|tokens| tokens.iter().map(|t| t.token.clone()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn elifarch_and_ifos_are_conditionals() {
        // `%elifarch`: `bar` is live on x86_64, `foo` is dead.
        let outcome = eval_own(
            "%if 0\nPrefer: foo\n%elifarch x86_64\nPrefer: bar\n%endif\n",
            "x86_64",
        );
        assert_eq!(prefer_tokens(&outcome), vec!["bar".to_string()]);
        assert!(has_finding(&outcome, PrjconfFindingKind::DeadBranch));
        // `%ifos linux` is true; `%ifos windows` is false.
        let outcome = eval_own("%ifos linux\nPrefer: foo\n%endif\n", "x86_64");
        assert_eq!(prefer_tokens(&outcome), vec!["foo".to_string()]);
        let outcome = eval_own("%ifos windows\nPrefer: foo\n%endif\n", "x86_64");
        assert!(prefer_tokens(&outcome).is_empty());
        assert!(has_finding(&outcome, PrjconfFindingKind::DeadBranch));
        // `%elifnarch` is not a conditional: the server ignores it,
        // so the `%if` group is not disturbed by it.
        let outcome = eval_own(
            "%if 1\nPrefer: foo\n%elifnarch x86_64\nPrefer: bar\n%endif\n",
            "x86_64",
        );
        assert_eq!(
            prefer_tokens(&outcome),
            vec!["foo".to_string(), "bar".to_string()]
        );
        assert!(!has_finding(&outcome, PrjconfFindingKind::ParseError));
    }

    #[test]
    fn std_macros_seed_ifarch_and_target_cpu() {
        // `%ifarch %{ix86}` matches i586 but not x86_64.
        let outcome = eval_own("%ifarch %{ix86}\nPrefer: foo\n%endif\n", "i586");
        assert_eq!(prefer_tokens(&outcome), vec!["foo".to_string()]);
        let outcome = eval_own("%ifarch %{ix86}\nPrefer: foo\n%endif\n", "x86_64");
        assert!(prefer_tokens(&outcome).is_empty());
        // `%{_target_cpu}` is the repository architecture.
        let outcome = eval_own(
            "%if %{_target_cpu} == \"x86_64\"\nPrefer: foo\n%endif\n",
            "x86_64",
        );
        assert_eq!(prefer_tokens(&outcome), vec!["foo".to_string()]);
    }

    #[test]
    fn trailing_comment_words_are_not_resolution_checked() {
        let mut outcome = eval_own("Prefer: realpkg # note\n", "x86_64");
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/home:test",
            "<directory><entry name=\"realpkg\"/></directory>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let path_projects: HashSet<String> = ["home:test".to_string()].into_iter().collect();
        let mut package_cache = HashMap::new();
        let mut owner_cache = HashMap::new();
        client.prjconf_resolve_and_shadow(
            &mut outcome,
            "home:test",
            "standard",
            &path_projects,
            &mut package_cache,
            &mut owner_cache,
            &mut OwnerLookupBudget::new(),
        );
        // `realpkg` resolves and `note` (a comment word) is not flagged.
        assert!(!has_finding(
            &outcome,
            PrjconfFindingKind::UnresolvableTarget
        ));
    }

    #[test]
    fn trailing_comment_hash_is_not_a_duplicate() {
        let outcome = eval_own("Prefer: foo # note\nPrefer: bar # note\n", "x86_64");
        assert!(!has_finding(&outcome, PrjconfFindingKind::DuplicateToken));
    }

    #[test]
    fn apply_plan_preserves_a_trailing_comment() {
        let own = "Prefer: foo foo # keep me\n";
        let mut plan = EvalPlan::default();
        plan.duplicate_tokens.insert((1, "foo".to_string()), 1);
        let (cleaned, removed) = apply_plan(own, &plan);
        assert_eq!(cleaned, "Prefer: foo # keep me\n");
        assert!(removed.is_empty());
    }

    #[test]
    fn ignore_and_conflict_resolve_the_package_part() {
        let mut outcome = eval_own("Ignore: realpkg:repo\nConflict: realpkg:other\n", "x86_64");
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/home:test",
            "<directory><entry name=\"realpkg\"/></directory>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let path_projects: HashSet<String> = ["home:test".to_string()].into_iter().collect();
        let mut package_cache = HashMap::new();
        let mut owner_cache = HashMap::new();
        client.prjconf_resolve_and_shadow(
            &mut outcome,
            "home:test",
            "standard",
            &path_projects,
            &mut package_cache,
            &mut owner_cache,
            &mut OwnerLookupBudget::new(),
        );
        assert!(!has_finding(
            &outcome,
            PrjconfFindingKind::UnresolvableTarget
        ));
        // A missing package part is flagged by its first part only.
        let mut outcome = eval_own("Ignore: missing:repo\n", "x86_64");
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:test",
                "<directory></directory>",
            )
            .with_get(
                "https://api.opensuse.org/search/owner?binary=missing",
                "<collection matches=\"0\"></collection>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let mut package_cache = HashMap::new();
        let mut owner_cache = HashMap::new();
        client.prjconf_resolve_and_shadow(
            &mut outcome,
            "home:test",
            "standard",
            &path_projects,
            &mut package_cache,
            &mut owner_cache,
            &mut OwnerLookupBudget::new(),
        );
        assert!(has_finding(
            &outcome,
            PrjconfFindingKind::UnresolvableTarget
        ));
        assert!(outcome
            .findings
            .iter()
            .any(|f| f.message.contains("`missing`")));
        assert!(!outcome
            .findings
            .iter()
            .any(|f| f.message.contains("missing:repo")));
    }

    #[test]
    fn dead_branch_message_names_the_evaluation() {
        let outcome = eval_own("%if 0\nPrefer: foo\n%endif\n", "x86_64");
        let finding = outcome
            .findings
            .iter()
            .find(|f| f.kind == PrjconfFindingKind::DeadBranch)
            .expect("a dead-branch finding");
        assert!(
            finding
                .message
                .contains("repository standard on architecture x86_64"),
            "{}",
            finding.message
        );
    }

    #[test]
    fn fromscratch_after_a_define_or_an_if_is_not_an_error() {
        // `%` lines are not content for the `FromScratch:` rule.
        let outcome = eval_own("%define foo bar\nFromScratch:\n", "x86_64");
        assert!(!has_finding(&outcome, PrjconfFindingKind::ParseError));
        let outcome = eval_own("%if 1\nFromScratch:\n%endif\n", "x86_64");
        assert!(!has_finding(&outcome, PrjconfFindingKind::ParseError));
        // Consecutive `FromScratch:` lines are all fine.
        let outcome = eval_own("FromScratch:\nFromScratch:\n", "x86_64");
        assert!(!has_finding(&outcome, PrjconfFindingKind::ParseError));
    }

    /// A two-section buildconfig (inherited + own) for the clean
    /// refusal tests: the meta paths both projects, the buildconfig is
    /// the crafted concatenation. Returns the stubbed HTTP transport;
    /// the test builds its client from it.
    fn clean_two_section_http(inherited: &str, own: &str) -> FakeHttp {
        let buildconfig = format!(
            "### from openSUSE:Factory\n%define _repository standard\n%define _project openSUSE:Factory\n{inherited}### from home:test\n%define _repository standard\n%define _project home:test\n%define _is_this_project 1\n{own}"
        );
        let meta = "<project name=\"home:test\"><repository name=\"standard\"><arch>x86_64</arch><path project=\"openSUSE:Factory\" repository=\"standard\"/><path project=\"home:test\" repository=\"standard\"/></repository></project>";
        FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:test/_meta",
                meta,
            )
            .with_get("https://api.opensuse.org/source/home:test/_config", own)
            .with_get(
                "https://api.opensuse.org/build/home:test/standard/_buildconfig",
                &buildconfig,
            )
            .with_get(
                "https://api.opensuse.org/public/source/openSUSE:Factory",
                "<directory><entry name=\"foo\"/></directory>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:test",
                "<directory></directory>",
            )
    }

    #[test]
    fn clean_does_not_refuse_on_an_inherited_stray_endif() {
        // A stray `%endif` in the inherited config is a parse error
        // there, but it does not corrupt the own evaluation.
        let http = clean_two_section_http("%endif\n", "Prefer: !never\n");
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(true);
        let result = client
            .clean_project_config("home:test", &dry, true)
            .expect("an inherited stray %endif does not refuse a clean");
        assert!(!result.applied);
        assert_eq!(result.removed_lines, vec![1]);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn clean_refuses_on_an_inherited_unclosed_if() {
        // An unclosed `%if` in the inherited config swallows the own
        // config's lines, so the evaluation is unreliable.
        let http = clean_two_section_http("%if 1\nPrefer: foo\n", "Prefer: !never\n");
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(true);
        let err = client
            .clean_project_config("home:test", &dry, false)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ValidationFailed);
    }

    fn clean_http(own: &str, meta: &str, buildconfigs: &[(&str, &str)]) -> FakeHttp {
        let mut http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:test/_meta",
                meta,
            )
            .with_get("https://api.opensuse.org/source/home:test/_config", own)
            .with_get(
                "https://api.opensuse.org/public/source/home:test",
                "<directory><entry name=\"foo\"/></directory>",
            );
        for (repo, buildconfig) in buildconfigs {
            http = http.with_get(
                &format!("https://api.opensuse.org/build/home:test/{repo}/_buildconfig"),
                buildconfig,
            );
        }
        http
    }

    fn one_repo_buildconfig(own: &str) -> String {
        format!(
            "### from home:test\n%define _repository standard\n%define _project home:test\n%define _is_this_project 1\n{own}"
        )
    }

    #[test]
    fn clean_keeps_crlf_terminators_and_tabs() {
        // A CRLF config stays CRLF: the rewrite splices byte
        // ranges, it never re-joins lines.
        let own = "Prefer:\tfoo\r\nPrefer: !bar\r\n";
        let meta = "<project name=\"home:test\"><repository name=\"standard\"><arch>x86_64</arch><path project=\"home:test\" repository=\"standard\"/></repository></project>";
        let http = clean_http(own, meta, &[("standard", &one_repo_buildconfig(own))]).with_put(
            "https://api.opensuse.org/source/home:test/_config",
            "<status code=\"ok\"/>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let live = DryRun::new(false);
        let result = client
            .clean_project_config("home:test", &live, true)
            .unwrap();
        assert!(result.applied);
        assert_eq!(result.removed_lines, vec![2]);
        assert_eq!(result.cleaned, "Prefer:\tfoo\r\n");
        assert_eq!(http.put_calls()[0].1, "Prefer:\tfoo\r\n");
    }

    #[test]
    fn clean_removes_a_dead_token_without_rebuilding_the_line() {
        // The duplicate token goes with one adjacent whitespace
        // run; the tab layout of the survivors is kept.
        let own = "Prefer:\tfoo\tfoo\tbar\n";
        let meta = "<project name=\"home:test\"><repository name=\"standard\"><arch>x86_64</arch><path project=\"home:test\" repository=\"standard\"/></repository></project>";
        let http = clean_http(own, meta, &[("standard", &one_repo_buildconfig(own))]);
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(true);
        let result = client
            .clean_project_config("home:test", &dry, false)
            .unwrap();
        assert_eq!(result.removed_lines, Vec::<usize>::new());
        assert_eq!(result.cleaned, "Prefer:\tfoo\tbar\n");
    }

    #[test]
    fn continuation_line_holding_a_prefer_is_not_a_duplicate() {
        // The second physical line belongs to the `%define` body,
        // not to a directive: only one real `Prefer: x` line runs,
        // so there is no duplicate to flag or clean.
        let own = "%define greeting Prefer: x \\\nPrefer: x\nPrefer: x\n";
        let outcome = eval_own(own, "x86_64");
        assert!(
            outcome
                .findings
                .iter()
                .all(|f| f.kind != PrjconfFindingKind::DuplicateToken),
            "unexpected duplicate: {:?}",
            outcome.findings
        );
        assert!(outcome.plan.duplicate_tokens.is_empty());
    }

    #[test]
    fn deep_nesting_is_unevaluatable_not_a_stack_overflow() {
        let own = format!("%if {}0 == 0\nPrefer: foo\n%endif\n", "(".repeat(65));
        let outcome = eval_own(&own, "x86_64");
        assert!(
            outcome
                .skipped
                .iter()
                .any(|s| s.reason.contains("could not be evaluated")),
            "no unevaluatable skip: {:?}",
            outcome.skipped
        );
        let own = format!("%if {}1\nPrefer: foo\n%endif\n", "!".repeat(65));
        let outcome = eval_own(&own, "x86_64");
        assert!(
            outcome
                .skipped
                .iter()
                .any(|s| s.reason.contains("could not be evaluated")),
            "no unevaluatable skip: {:?}",
            outcome.skipped
        );
    }

    #[test]
    fn empty_expansion_fan_out_is_unevaluatable_not_a_hang() {
        // `%define a0` is empty; each `aN` is twenty `%a(N-1)`
        // references. Without the step budget the final expansion
        // would fan out to 20^16 calls; the budget yields
        // unevaluatable instead.
        let mut own = String::from("%define a0\n");
        for n in 1..=16 {
            own.push_str(&format!(
                "%define a{n} {}\n",
                format!("%a{} ", n - 1).repeat(20)
            ));
        }
        own.push_str("%if \"%a16\" == \"x\"\nPrefer: foo\n%endif\n");
        let outcome = eval_own(&own, "x86_64");
        assert!(
            outcome
                .skipped
                .iter()
                .any(|s| s.reason.contains("could not be evaluated")),
            "no unevaluatable skip: {:?}",
            outcome.skipped
        );
    }

    #[test]
    fn unicode_tokens_are_spliced_on_char_boundaries() {
        let own = "Prefer: föö föö\nPrefer: !bár\n";
        let meta = "<project name=\"home:test\"><repository name=\"standard\"><arch>x86_64</arch><path project=\"home:test\" repository=\"standard\"/></repository></project>";
        let http = clean_http(own, meta, &[("standard", &one_repo_buildconfig(own))]);
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(true);
        let result = client
            .clean_project_config("home:test", &dry, true)
            .unwrap();
        assert_eq!(result.removed_lines, vec![2]);
        assert_eq!(result.cleaned, "Prefer: föö\n");
    }

    #[test]
    fn clean_removes_a_dead_group_whose_if_line_continues() {
        // The `%if` line itself spans two physical lines: the
        // whole group goes, continuation line included.
        let own = "%if 0 == 1 \\\n  || 0 == 1\nPrefer: !never-added\n%endif\n";
        let meta = "<project name=\"home:test\"><repository name=\"standard\"><arch>x86_64</arch><path project=\"home:test\" repository=\"standard\"/></repository></project>";
        let http = clean_http(own, meta, &[("standard", &one_repo_buildconfig(own))]);
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(true);
        let result = client
            .clean_project_config("home:test", &dry, true)
            .unwrap();
        assert_eq!(result.removed_lines, vec![1, 2, 3, 4]);
        assert_eq!(result.cleaned, "");
    }

    #[test]
    fn clean_default_removes_only_path_independent_proofs() {
        let own = "Prefer: foo foo\nSubstitute: a b\nSubstitute: a c\nPrefer: !never-added\n";
        let meta = "<project name=\"home:test\"><repository name=\"standard\"><arch>x86_64</arch><path project=\"home:test\" repository=\"standard\"/></repository></project>";
        let http = clean_http(own, meta, &[("standard", &one_repo_buildconfig(own))]);
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(true);
        // By default: the same-line duplicate and the
        // unconditionally overridden `Substitute` go; the dead
        // removal — a proof about this project's evaluations —
        // stays.
        let result = client
            .clean_project_config("home:test", &dry, false)
            .unwrap();
        assert_eq!(
            result.cleaned,
            "Prefer: foo\nSubstitute: a c\nPrefer: !never-added\n"
        );
        // With the opt-in the dead removal goes too.
        let result = client
            .clean_project_config("home:test", &dry, true)
            .unwrap();
        assert_eq!(result.removed_lines, vec![2, 4]);
        assert_eq!(result.cleaned, "Prefer: foo\nSubstitute: a c\n");
    }

    #[test]
    fn clean_default_keeps_a_conditionally_overridden_substitute() {
        let own = "Substitute: a b\n%if 1 == 1\nSubstitute: a c\n%endif\n";
        let meta = "<project name=\"home:test\"><repository name=\"standard\"><arch>x86_64</arch><path project=\"home:test\" repository=\"standard\"/></repository></project>";
        let http = clean_http(own, meta, &[("standard", &one_repo_buildconfig(own))]);
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(true);
        let result = client
            .clean_project_config("home:test", &dry, false)
            .unwrap();
        assert_eq!(result.cleaned, own);
        let result = client
            .clean_project_config("home:test", &dry, true)
            .unwrap();
        assert_eq!(result.removed_lines, vec![1]);
        assert_eq!(result.cleaned, "%if 1 == 1\nSubstitute: a c\n%endif\n");
    }

    #[test]
    fn clean_across_repositories_keeps_a_line_dead_in_only_one() {
        // `Prefer: !foo` removes `foo` in the standard evaluation
        // (where the `%if` branch adds it) but is a dead removal in
        // the other evaluation (where the branch is dead): dead in
        // one repository only, so no clean removes it. The
        // unconditional dead removal is dead in both and goes with
        // the opt-in.
        let own = "%if \"%{_repository}\" == \"standard\"\nPrefer: foo\n%endif\nPrefer: !foo\nPrefer: !never-added\n";
        let meta = "<project name=\"home:test\"><repository name=\"standard\"><arch>x86_64</arch><path project=\"home:test\" repository=\"standard\"/></repository><repository name=\"other\"><arch>x86_64</arch><path project=\"home:test\" repository=\"other\"/></repository></project>";
        let standard = format!(
            "### from home:test\n%define _repository standard\n%define _project home:test\n%define _is_this_project 1\n{own}"
        );
        let other = format!(
            "### from home:test\n%define _repository other\n%define _project home:test\n%define _is_this_project 1\n{own}"
        );
        let http = clean_http(own, meta, &[("standard", &standard), ("other", &other)]);
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(true);
        let result = client
            .clean_project_config("home:test", &dry, true)
            .unwrap();
        assert_eq!(result.removed_lines, vec![5]);
        assert_eq!(
            result.cleaned,
            "%if \"%{_repository}\" == \"standard\"\nPrefer: foo\n%endif\nPrefer: !foo\n"
        );
        let result = client
            .clean_project_config("home:test", &dry, false)
            .unwrap();
        assert!(result.removed_lines.is_empty());
        assert_eq!(result.cleaned, own);
    }

    #[test]
    fn branch_dead_in_one_arch_only_survives_clean_and_validate() {
        // The branch is false on aarch64 only: not dead in every
        // evaluation, so it survives `clean` even with the opt-in
        // and produces no `dead_branch` finding in `validate`.
        let own = "%if %{_target_cpu} == \"x86_64\"\nPrefer: !never-added\n%endif\n";
        let meta = "<project name=\"home:test\"><repository name=\"standard\"><arch>x86_64</arch><arch>aarch64</arch><path project=\"home:test\" repository=\"standard\"/></repository></project>";
        let http = clean_http(own, meta, &[("standard", &one_repo_buildconfig(own))]);
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let validation = client.validate_project_config("home:test").unwrap();
        assert!(
            validation
                .findings
                .iter()
                .all(|f| f.kind != PrjconfFindingKind::DeadBranch),
            "unexpected dead_branch: {:?}",
            validation.findings
        );
        assert!(
            validation
                .findings
                .iter()
                .any(|f| f.kind == PrjconfFindingKind::DeadRemoval),
            "expected the x86_64 dead removal: {:?}",
            validation.findings
        );
        let dry = DryRun::new(true);
        let result = client
            .clean_project_config("home:test", &dry, true)
            .unwrap();
        assert!(result.removed_lines.is_empty());
        assert_eq!(result.cleaned, own);
    }

    #[test]
    fn if_semantics_quoted_zero_is_truthy() {
        // `"%{x}" == "0"` compares two non-empty strings: true,
        // so the branch runs and the dead removal is found.
        let own = "%define x 0\n%if \"%{x}\" == \"0\"\nPrefer: !never-added\n%endif\n";
        let outcome = eval_own(own, "x86_64");
        assert!(
            outcome
                .findings
                .iter()
                .any(|f| f.kind == PrjconfFindingKind::DeadRemoval),
            "expected a dead removal: {:?}",
            outcome.findings
        );
    }

    #[test]
    fn if_semantics_non_canonical_input_is_unevaluatable() {
        for expr in ["-x == 1", "15.6 == 1"] {
            let own = format!("%if {expr}\nPrefer: foo\n%endif\n");
            let outcome = eval_own(&own, "x86_64");
            assert!(
                outcome
                    .skipped
                    .iter()
                    .any(|s| s.reason.contains("could not be evaluated")),
                "{expr}: no unevaluatable skip: {:?}",
                outcome.skipped
            );
        }
    }

    #[test]
    fn if_directives_are_case_sensitive() {
        // `%IF` / `%Endif` are not conditionals: no `%if` is ever
        // opened, so nothing is unclosed — the bare `%Endif` is
        // just an unexpandable macro reference, skipped, not a
        // parse error.
        let own = "%IF 0\nPrefer: foo\n%Endif\n";
        let outcome = eval_own(own, "x86_64");
        assert!(outcome.findings.is_empty(), "{:?}", outcome.findings);
        assert_eq!(outcome.skipped.len(), 1, "{:?}", outcome.skipped);
        assert_eq!(outcome.skipped[0].line, Some(3));
        assert!(
            outcome.skipped[0].reason.contains("could not be expanded"),
            "{:?}",
            outcome.skipped
        );
        // `%if 0` closed only by `%Endif` is unclosed: `%Endif`
        // does not close the group.
        let own = "%if 0\nPrefer: foo\n%Endif\n";
        let outcome = eval_own(own, "x86_64");
        assert!(
            outcome
                .findings
                .iter()
                .any(|f| f.kind == PrjconfFindingKind::UnclosedIf
                    && f.severity == PrjconfSeverity::Error),
            "expected an unclosed_if error: {:?}",
            outcome.findings
        );
    }

    #[test]
    fn define_directive_is_case_sensitive() {
        // `%DEFINE` does not define `foo`: the reference in the
        // `Prefer` line cannot be expanded.
        let own = "%DEFINE foo bar\nPrefer: %{foo}\n";
        let outcome = eval_own(own, "x86_64");
        assert!(
            outcome
                .skipped
                .iter()
                .any(|s| s.reason.contains("could not be expanded")),
            "no unexpandable skip: {:?}",
            outcome.skipped
        );
    }

    #[test]
    fn owner_lookup_cap_reports_one_skip_and_guesses_nothing() {
        // 150 distinct unknown packages: the first 100 spend the
        // lookup budget (each lookup fails and is left alone); the
        // rest are never looked up, recorded once as a cap skip,
        // and never reported unresolvable.
        let mut own = String::new();
        for n in 0..150 {
            own.push_str(&format!("Prefer: pkg{n:03}\n"));
        }
        let meta = "<project name=\"home:test\"><repository name=\"standard\"><arch>x86_64</arch><path project=\"home:test\" repository=\"standard\"/></repository></project>";
        let http = clean_http(&own, meta, &[("standard", &one_repo_buildconfig(&own))]);
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let validation = client.validate_project_config("home:test").unwrap();
        assert_eq!(
            validation
                .skipped
                .iter()
                .filter(|s| s.reason.contains("owner lookups capped at 100"))
                .count(),
            1,
            "expected one cap skip: {:?}",
            validation.skipped
        );
        assert!(
            validation
                .findings
                .iter()
                .all(|f| f.kind != PrjconfFindingKind::UnresolvableTarget),
            "unexpected unresolvable_target: {:?}",
            validation.findings
        );
    }

    #[test]
    fn text_rendering_escapes_control_characters() {
        let finding = PrjconfFinding {
            kind: PrjconfFindingKind::UnknownKeyword,
            severity: PrjconfSeverity::Warning,
            repository: None,
            project: "home:test".to_string(),
            line: Some(1),
            message: "evil \u{1b}[31mred\u{07} bell".to_string(),
        };
        let line = prjconf_finding_line(&finding);
        assert!(!line.contains('\u{1b}'), "{line:?}");
        assert!(!line.contains('\u{07}'), "{line:?}");
        assert!(line.contains("\\u{1b}"), "{line:?}");
        assert!(line.contains("\\u{7}"), "{line:?}");
    }

    #[test]
    fn clean_refuses_when_the_config_changes_before_the_write() {
        // The re-read just before the PUT sees a changed config:
        // the clean refuses and no PUT happens.
        let own = "Prefer: foo\nPrefer: !bar\n";
        let changed = "Prefer: foo\nPrefer: !bar\nPrefer: baz\n";
        let meta = "<project name=\"home:test\"><repository name=\"standard\"><arch>x86_64</arch><path project=\"home:test\" repository=\"standard\"/></repository></project>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:test/_meta",
                meta,
            )
            .with_get_sequence(
                "https://api.opensuse.org/source/home:test/_config",
                &[own, changed],
            )
            .with_get(
                "https://api.opensuse.org/build/home:test/standard/_buildconfig",
                &one_repo_buildconfig(own),
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:test",
                "<directory><entry name=\"foo\"/></directory>",
            )
            .with_put(
                "https://api.opensuse.org/source/home:test/_config",
                "<status code=\"ok\"/>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let live = DryRun::new(false);
        let err = client
            .clean_project_config("home:test", &live, true)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ValidationFailed);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn inherited_dead_branch_finding_names_the_project_not_a_line() {
        // A dead branch in the inherited config carries no own
        // line: the finding names the project instead.
        let outcome = eval_two_section("%if 0 == 1\nPrefer: foo\n%endif\n", "", "x86_64");
        let finding = outcome
            .findings
            .iter()
            .find(|f| f.kind == PrjconfFindingKind::DeadBranch)
            .expect("expected a dead_branch finding");
        assert_eq!(finding.line, None);
        assert!(
            finding.message.contains("openSUSE:Factory"),
            "{}",
            finding.message
        );
    }
}
