use std::str::FromStr;
use thiserror::Error;

const SUBJECT_MAX_LEN: usize = 50;
const BODY_LINE_MAX_LEN: usize = 72;

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
        "Subject too long ({len} > 50).\nGiven current type/scope/bang, you have ~{budget} chars for --summary.\nSubject: {subject}"
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

        for line in body.lines() {
            if !line.is_empty() && line.len() > BODY_LINE_MAX_LEN {
                return Err(ValidationError::BodyLineTooLong {
                    len: line.len(),
                    line: line.to_string(),
                });
            }
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
