use std::cmp::Ordering;
use std::collections::HashSet;
use std::str::FromStr;
use strum::IntoEnumIterator;
use thiserror::Error;

const SUBJECT_MAX_LEN: usize = 70;
const BODY_LINE_MAX_LEN: usize = 72;

// ============================================================================
// Verb - Action verbs for commit body bullet points (transformation priority)
// ============================================================================

#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
    strum::AsRefStr,
    strum::Display,
    strum::EnumIter,
    strum::EnumString,
)]
#[repr(u8)]
pub enum Verb {
    Remove = 0,
    Fix = 1,
    Refactor = 2,
    Move = 3,
    Rename = 4,
    Change = 5,
    Add = 6,
    Upgrade = 7,
    Downgrade = 8,
}

impl Verb {
    pub fn allowed_list() -> String {
        Self::iter()
            .map(|verb| verb.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

// ============================================================================
// Action - A single bullet point action in the commit body
// ============================================================================

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Action {
    verb: Verb,
    detail: String,
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.verb, self.detail)
    }
}

impl Action {
    pub fn new(verb: Verb, detail: &str) -> Result<Self, ValidationError> {
        let detail = detail.trim();
        if detail.is_empty() {
            return Err(ValidationError::ActionDetailEmpty);
        }

        // Ensure detail ends with a period
        let detail = if detail.ends_with('.') {
            detail.to_owned()
        } else {
            format!("{}.", detail)
        };

        Ok(Self { verb, detail })
    }

    pub fn parse(raw: &str) -> Result<Self, ValidationError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(ValidationError::ActionEmpty);
        }

        let (verb_raw, detail_raw) = trimmed
            .split_once(' ')
            .ok_or(ValidationError::ActionMissingDetail)?;

        let verb: Verb = verb_raw.parse().map_err(|_| ValidationError::ActionInvalidVerb {
            word: verb_raw.to_owned(),
            allowed: Verb::allowed_list(),
        })?;

        Self::new(verb, detail_raw)
    }

    pub fn verb(&self) -> Verb {
        self.verb
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl PartialOrd for Action {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Action {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.verb.cmp(&other.verb) {
            Ordering::Equal => self.detail.cmp(&other.detail),
            ordering @ (Ordering::Less | Ordering::Greater) => ordering,
        }
    }
}

// ============================================================================
// ActionList - A sorted, deduplicated list of actions
// ============================================================================

#[derive(Clone, Debug)]
pub struct ActionList {
    actions: Vec<Action>,
}

impl ActionList {
    pub fn new(actions: Vec<Action>) -> Self {
        let mut dedup: HashSet<Action> = HashSet::from_iter(actions);
        let mut actions: Vec<_> = dedup.drain().collect();
        actions.sort();
        Self { actions }
    }

    pub fn actions(&self) -> &[Action] {
        &self.actions
    }

    pub fn body(&self) -> String {
        self.actions
            .iter()
            .flat_map(|action| self.to_body_lines(action))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn to_body_lines(&self, action: &Action) -> Vec<String> {
        let mut result = Vec::new();
        let text = action.to_string();

        for raw_line in text.lines() {
            let normalized = raw_line.split_whitespace().collect::<Vec<_>>().join(" ");
            if normalized.is_empty() {
                continue;
            }
            let prefix = if result.is_empty() { "- " } else { "  " };
            result.push(format!("{prefix}{normalized}"));
        }

        result
    }

    /// Parse a body string into an ActionList, validating each action
    pub fn parse(body: &str) -> Result<Self, ValidationError> {
        let mut actions = Vec::new();
        let mut current_action_lines: Vec<&str> = Vec::new();

        for line in body.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Check if this is a new bullet point
            if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
                // Process previous action if any
                if !current_action_lines.is_empty() {
                    let action_text = current_action_lines.join(" ");
                    let action_text = action_text.strip_prefix("- ")
                        .or_else(|| action_text.strip_prefix("* "))
                        .unwrap_or(&action_text);
                    actions.push(Action::parse(action_text)?);
                    current_action_lines.clear();
                }
                current_action_lines.push(trimmed);
            } else if !current_action_lines.is_empty() {
                // Continuation line
                current_action_lines.push(trimmed);
            } else {
                // Line doesn't start with bullet and no current action
                return Err(ValidationError::BodyMustStartWithBulletPoint {
                    line: line.to_string(),
                });
            }
        }

        // Process last action
        if !current_action_lines.is_empty() {
            let action_text = current_action_lines.join(" ");
            let action_text = action_text.strip_prefix("- ")
                .or_else(|| action_text.strip_prefix("* "))
                .unwrap_or(&action_text);
            actions.push(Action::parse(action_text)?);
        }

        if actions.is_empty() {
            return Err(ValidationError::BodyEmpty);
        }

        Ok(Self::new(actions))
    }
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum CommitType {
    Fix,
    Feat,
    Docs,
    Style,
    Refactor,
    Perf,
    Test,
    Build,
    Chore,
    Ci,
    Revert,
}

impl CommitType {
    pub const fn as_str(&self) -> &'static str {
        match self {
            CommitType::Fix => "fix",
            CommitType::Feat => "feat",
            CommitType::Docs => "docs",
            CommitType::Style => "style",
            CommitType::Refactor => "refactor",
            CommitType::Perf => "perf",
            CommitType::Test => "test",
            CommitType::Build => "build",
            CommitType::Chore => "chore",
            CommitType::Ci => "ci",
            CommitType::Revert => "revert",
        }
    }

    pub fn allowed_list() -> String {
        [
            "fix", "feat", "docs", "style", "refactor", "perf", "test", "build", "chore", "ci",
            "revert",
        ]
        .join(" ")
    }
}

impl std::fmt::Display for CommitType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for CommitType {
    type Err = ValidationError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim() {
            "fix" => Ok(CommitType::Fix),
            "feat" => Ok(CommitType::Feat),
            "docs" => Ok(CommitType::Docs),
            "style" => Ok(CommitType::Style),
            "refactor" => Ok(CommitType::Refactor),
            "perf" => Ok(CommitType::Perf),
            "test" => Ok(CommitType::Test),
            "build" => Ok(CommitType::Build),
            "chore" => Ok(CommitType::Chore),
            "ci" => Ok(CommitType::Ci),
            "revert" => Ok(CommitType::Revert),
            other => Err(ValidationError::CommitTypeInvalid {
                raw: other.to_string(),
                allowed: CommitType::allowed_list(),
            }),
        }
    }
}

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("Commit type must be one of {allowed} - got '{raw}'")]
    CommitTypeInvalid { raw: String, allowed: String },

    #[error("Summary is required")]
    SummaryEmpty,

    #[error("Summary must be a single line")]
    SummaryMultiline,

    #[error("Summary should not end with a period")]
    SummaryEndsWithPeriod,

    #[error("Scope must be a single line")]
    ScopeMultiline,

    #[error("Scope must not contain a closing parenthesis")]
    ScopeHasClosingParen,

    #[error("Scope cannot be empty when explicitly provided")]
    ScopeEmpty,

    #[error("Body cannot be empty when explicitly provided")]
    BodyEmpty,

    #[error("Breaking note cannot be empty")]
    BreakingNoteEmpty,

    #[error("Breaking note must be a single line")]
    BreakingNoteMultiline,

    #[error("Body line too long ({len} chars). Max 72 characters per line.\nLine: {line}")]
    BodyLineTooLong { len: usize, line: String },

    #[error(
        "Subject too long ({len} > 70).\nGiven current type/scope/bang, you have ~{budget} chars for --summary.\nSubject: {subject}"
    )]
    SubjectTooLong {
        len: usize,
        budget: usize,
        subject: String,
    },

    #[error("Commit message subject line is required")]
    MessageSubjectMissing,

    #[error(
        "Commit message subject must be formatted like 'type(scope)!: summary' or 'type: summary' - got '{subject}'"
    )]
    MessageSubjectInvalidFormat { subject: String },

    #[error("When a commit message has body/footers, it must include a blank line after the subject")]
    MessageMissingBlankLineAfterSubject,

    #[error("BREAKING CHANGE footer must be the final non-comment line")]
    MessageBreakingFooterNotLast,

    #[error("BREAKING CHANGE footer requires '!' in the subject")]
    MessageBreakingFooterMissingBang,

    #[error("Subject uses '!' but no BREAKING CHANGE footer was found")]
    MessageBangWithoutBreakingFooter,

    #[error("Body must start with bullet points (- or *).\nLine: {line}")]
    BodyMustStartWithBulletPoint { line: String },

    #[error("Action cannot be empty")]
    ActionEmpty,

    #[error("Action must start with one of: {allowed}\nGot: '{word}'")]
    ActionInvalidVerb { word: String, allowed: String },

    #[error("Action must have a description after the verb")]
    ActionMissingDetail,

    #[error("Action description cannot be empty")]
    ActionDetailEmpty,

    #[error("Body is not in canonical form.\nExpected:\n{expected}\n\nGot:\n{actual}")]
    BodyNotCanonical { expected: String, actual: String },
}

#[derive(Debug, Clone)]
pub struct CommitSummary(String);

impl CommitSummary {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for CommitSummary {
    type Err = ValidationError;

    fn from_str(raw_summary: &str) -> Result<Self, Self::Err> {
        let trimmed = raw_summary.trim();
        if trimmed.is_empty() {
            return Err(ValidationError::SummaryEmpty);
        }
        if trimmed.contains('\n') || trimmed.contains('\r') {
            return Err(ValidationError::SummaryMultiline);
        }

        let summary = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");

        if summary.ends_with('.') {
            return Err(ValidationError::SummaryEndsWithPeriod);
        }

        Ok(CommitSummary(summary))
    }
}

#[derive(Debug, Clone)]
pub struct CommitScope(String);

impl CommitScope {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for CommitScope {
    type Err = ValidationError;

    fn from_str(raw_scope: &str) -> Result<Self, Self::Err> {
        let scope = raw_scope.trim();

        if scope.is_empty() {
            return Err(ValidationError::ScopeEmpty);
        }
        if scope.contains('\n') || scope.contains('\r') {
            return Err(ValidationError::ScopeMultiline);
        }
        if scope.contains(')') {
            return Err(ValidationError::ScopeHasClosingParen);
        }

        Ok(CommitScope(scope.to_string()))
    }
}

#[derive(Debug, Clone)]
pub struct CommitBody(String);

impl CommitBody {
    pub fn from_action_list(action_list: &ActionList) -> Self {
        Self(action_list.body())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for CommitBody {
    type Err = ValidationError;

    fn from_str(raw_body: &str) -> Result<Self, Self::Err> {
        let body = raw_body.trim();

        if body.is_empty() {
            return Err(ValidationError::BodyEmpty);
        }

        // Check line lengths
        for line in body.lines() {
            if !line.is_empty() && line.len() > BODY_LINE_MAX_LEN {
                return Err(ValidationError::BodyLineTooLong {
                    len: line.len(),
                    line: line.to_string(),
                });
            }
        }

        // Parse into ActionList (validates each action and sorts by priority)
        let action_list = ActionList::parse(body)?;

        // Round-trip: re-serialize and compare to ensure canonical form
        let canonical = action_list.body();
        if canonical != body {
            return Err(ValidationError::BodyNotCanonical {
                expected: canonical,
                actual: body.to_string(),
            });
        }

        Ok(CommitBody(body.to_string()))
    }
}

#[derive(Debug, Clone)]
pub struct BreakingNote(String);

impl BreakingNote {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for BreakingNote {
    type Err = ValidationError;

    fn from_str(raw_note: &str) -> Result<Self, Self::Err> {
        let note = raw_note.trim();
        if note.is_empty() {
            return Err(ValidationError::BreakingNoteEmpty);
        }
        if note.contains('\n') || note.contains('\r') {
            return Err(ValidationError::BreakingNoteMultiline);
        }
        Ok(BreakingNote(note.to_string()))
    }
}

#[derive(Debug, Clone)]
pub struct CommitSubject(String);

impl CommitSubject {
    pub fn new(
        commit_type: CommitType,
        scope: Option<&CommitScope>,
        summary: &CommitSummary,
        breaking_note: Option<&BreakingNote>,
    ) -> Result<Self, ValidationError> {
        let bang = if breaking_note.is_some() { "!" } else { "" };

        let subject = if let Some(scope) = scope {
            format!(
                "{}({}){}: {}",
                commit_type,
                scope.as_str(),
                bang,
                summary.as_str()
            )
        } else {
            format!("{}{}: {}", commit_type, bang, summary.as_str())
        };

        let subject_length = subject.len();
        if subject_length > SUBJECT_MAX_LEN {
            let prefix_without_summary = subject.replace(summary.as_str(), "");
            let budget = SUBJECT_MAX_LEN.saturating_sub(prefix_without_summary.len());
            return Err(ValidationError::SubjectTooLong {
                len: subject_length,
                budget,
                subject,
            });
        }

        Ok(CommitSubject(subject))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub fn new_commit_message(
    subject: &CommitSubject,
    body: Option<&CommitBody>,
    breaking_note: Option<&BreakingNote>,
) -> String {
    let mut message_parts = vec![subject.as_str().to_string()];

    if let Some(body) = body {
        message_parts.push(String::new());
        message_parts.push(body.as_str().to_string());
    }

    if let Some(breaking_note) = breaking_note {
        message_parts.push(String::new());
        message_parts.push(format!("BREAKING CHANGE: {}", breaking_note.as_str()));
    }

    message_parts.join("\n")
}

pub fn validate_commit_message(raw_message: &str) -> Result<(), ValidationError> {
    let mut lines: Vec<&str> = raw_message
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect();

    while matches!(lines.first(), Some(line) if line.trim().is_empty()) {
        lines.remove(0);
    }
    while matches!(lines.last(), Some(line) if line.trim().is_empty()) {
        lines.pop();
    }

    let subject_line = lines
        .first()
        .copied()
        .ok_or(ValidationError::MessageSubjectMissing)?
        .trim();

    if is_autosquash_subject(subject_line) {
        let rest = subject_line
            .split_once(' ')
            .map(|(_prefix, rest)| rest.trim())
            .unwrap_or("");
        if rest.is_empty() {
            return Err(ValidationError::MessageSubjectInvalidFormat {
                subject: subject_line.to_string(),
            });
        }
        return Ok(());
    }

    let (commit_type, scope, summary, has_bang) = parse_subject_line(subject_line)?;
    validate_subject_length(subject_line, summary.as_str())?;

    let rest = &lines[1..];
    if rest.is_empty() {
        if has_bang {
            return Err(ValidationError::MessageBangWithoutBreakingFooter);
        }
        return Ok(());
    }

    if !rest[0].trim().is_empty() {
        return Err(ValidationError::MessageMissingBlankLineAfterSubject);
    }

    let content = &rest[1..];
    if content.is_empty() {
        if has_bang {
            return Err(ValidationError::MessageBangWithoutBreakingFooter);
        }
        return Ok(());
    }

    let (body_lines, breaking_note) = split_body_and_breaking_footer(content)?;

    if let Some(breaking_note) = breaking_note.as_ref() {
        if !has_bang {
            return Err(ValidationError::MessageBreakingFooterMissingBang);
        }
        let _ = BreakingNote::from_str(breaking_note)?;
    } else if has_bang {
        return Err(ValidationError::MessageBangWithoutBreakingFooter);
    }

    if has_non_empty_body(body_lines) {
        let body_text = body_lines.join("\n");
        let _ = CommitBody::from_str(&body_text)?;
    }

    // Validate subject type/scope/summary by reconstructing. This ensures scope parsing rules
    // match the CLI rules (including ')' and newline checks).
    let scope = scope.as_ref().map(|s| CommitScope::from_str(s)).transpose()?;
    let breaking_note = breaking_note
        .as_ref()
        .map(|note| BreakingNote::from_str(note))
        .transpose()?;
    let _ = CommitSubject::new(commit_type, scope.as_ref(), &summary, breaking_note.as_ref())?;

    Ok(())
}

fn is_autosquash_subject(subject: &str) -> bool {
    subject.starts_with("fixup! ") || subject.starts_with("squash! ") || subject.starts_with("amend! ")
}

fn parse_subject_line(
    subject: &str,
) -> Result<(CommitType, Option<String>, CommitSummary, bool), ValidationError> {
    let (type_scope_bang, summary_raw) = subject
        .split_once(": ")
        .ok_or_else(|| ValidationError::MessageSubjectInvalidFormat {
            subject: subject.to_string(),
        })?;

    let mut prefix = type_scope_bang.trim();
    if prefix.is_empty() {
        return Err(ValidationError::MessageSubjectInvalidFormat {
            subject: subject.to_string(),
        });
    }

    let has_bang = prefix.ends_with('!');
    if has_bang {
        prefix = prefix.strip_suffix('!').unwrap_or(prefix).trim_end();
        if prefix.is_empty() {
            return Err(ValidationError::MessageSubjectInvalidFormat {
                subject: subject.to_string(),
            });
        }
    }

    let (commit_type, scope) = if prefix.ends_with(')') {
        let open = prefix.find('(').ok_or_else(|| ValidationError::MessageSubjectInvalidFormat {
            subject: subject.to_string(),
        })?;
        let raw_type = prefix[..open].trim();
        let raw_scope = &prefix[open + 1..prefix.len() - 1];
        if raw_type.is_empty() {
            return Err(ValidationError::MessageSubjectInvalidFormat {
                subject: subject.to_string(),
            });
        }
        let commit_type = <CommitType as FromStr>::from_str(raw_type)?;
        let scope = raw_scope.to_string();
        (commit_type, Some(scope))
    } else {
        let commit_type = <CommitType as FromStr>::from_str(prefix)?;
        (commit_type, None)
    };

    let summary = CommitSummary::from_str(summary_raw)?;
    Ok((commit_type, scope, summary, has_bang))
}

fn validate_subject_length(subject: &str, summary: &str) -> Result<(), ValidationError> {
    let len = subject.len();
    if len <= SUBJECT_MAX_LEN {
        return Ok(());
    }

    let prefix_without_summary = subject.replace(summary, "");
    let budget = SUBJECT_MAX_LEN.saturating_sub(prefix_without_summary.len());
    Err(ValidationError::SubjectTooLong {
        len,
        budget,
        subject: subject.to_string(),
    })
}

fn split_body_and_breaking_footer<'a>(
    content: &'a [&'a str],
) -> Result<(&'a [&'a str], Option<String>), ValidationError> {
    let breaking_indices: Vec<usize> = content
        .iter()
        .enumerate()
        .filter_map(|(idx, line)| {
            if line.trim_start().starts_with("BREAKING CHANGE:") {
                Some(idx)
            } else {
                None
            }
        })
        .collect();

    if breaking_indices.is_empty() {
        return Ok((content, None));
    }

    if breaking_indices.len() != 1 || breaking_indices[0] != content.len() - 1 {
        return Err(ValidationError::MessageBreakingFooterNotLast);
    }

    let line = content.last().copied().unwrap_or("").trim_end();
    let note_raw = line
        .trim_start()
        .strip_prefix("BREAKING CHANGE:")
        .unwrap_or("");
    let note = note_raw.strip_prefix(' ').unwrap_or(note_raw).trim();
    if note.is_empty() {
        return Err(ValidationError::BreakingNoteEmpty);
    }

    if content.len() == 1 {
        return Ok((&[], Some(note.to_string())));
    }

    let before = content[content.len() - 2];
    if !before.trim().is_empty() {
        return Err(ValidationError::MessageBreakingFooterNotLast);
    }

    let body_lines = &content[..content.len() - 2];
    Ok((body_lines, Some(note.to_string())))
}

fn has_non_empty_body(body_lines: &[&str]) -> bool {
    body_lines.iter().any(|line| !line.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_validation_rejects_period() {
        let err = CommitSummary::from_str("Do the thing.").unwrap_err();
        assert!(matches!(err, ValidationError::SummaryEndsWithPeriod));
    }

    #[test]
    fn validate_allows_subject_only() {
        validate_commit_message("fix: Handle empty input\n").unwrap();
    }

    #[test]
    fn validate_rejects_body_without_blank_line() {
        let err = validate_commit_message("fix: Handle empty input\nBody\n").unwrap_err();
        assert!(matches!(
            err,
            ValidationError::MessageMissingBlankLineAfterSubject
        ));
    }

    #[test]
    fn validate_allows_breaking_change_footer_with_bang() {
        let msg = "feat!: Change API\n\nBREAKING CHANGE: Old thing removed\n";
        validate_commit_message(msg).unwrap();
    }

    #[test]
    fn validate_rejects_breaking_change_footer_without_bang() {
        let msg = "feat: Change API\n\nBREAKING CHANGE: Old thing removed\n";
        let err = validate_commit_message(msg).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::MessageBreakingFooterMissingBang
        ));
    }

    #[test]
    fn validate_allows_fixup_commits() {
        validate_commit_message("fixup! feat: Add feature\n").unwrap();
    }
}
