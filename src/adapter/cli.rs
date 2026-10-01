//! Command-line transport for choosing a review source.
//!
//! This module translates Clap's external argument representation into the
//! domain `DiffRequest`; it does not load source data or start the terminal.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::domain::diff::{
    DiffRequest, DiffSource, GitComparison, PatchInput, split_revision_range,
};

/// Review diffs in the terminal without changing the repository.
#[derive(Debug, Parser)]
#[command(version, about)]
#[command(
    after_help = "Canonical forms:\n  revia git changes\n  revia git staged\n  revia git unstaged\n  revia git revision <REVISION>\n  revia git range <A..B|A...B>\n  revia patch <FILE|->"
)]
pub struct Args {
    /// Git repository to inspect (defaults to the current directory).
    #[arg(long, global = true, default_value = ".")]
    pub repo: PathBuf,

    /// Number of unchanged lines surrounding each hunk.
    #[arg(short = 'U', long, global = true, default_value_t = 3)]
    pub context: usize,

    /// Write the raw loaded diff to stdout instead of opening the TUI.
    #[arg(long, global = true)]
    pub print: bool,

    #[command(subcommand)]
    source: Option<SourceCommand>,

    /// Shortcut: a revision, a revision range, or `-` for a patch on stdin.
    #[arg(value_parser = shortcut_input)]
    shortcut: Option<String>,
}

#[derive(Debug, Subcommand)]
enum SourceCommand {
    /// Load a comparison from Git.
    Git {
        #[command(subcommand)]
        comparison: GitCommand,
    },
    /// Load a patch file, or use `-` to read a patch from stdin.
    Patch { input: String },
}

#[derive(Debug, Subcommand)]
enum GitCommand {
    /// Review all current changes relative to HEAD, including untracked files.
    Changes,
    /// Review changes staged in the index.
    Staged,
    /// Review tracked changes not staged in the index.
    Unstaged,
    /// Review the patch introduced by one commit-ish revision.
    Revision {
        #[arg(value_parser = revision_input)]
        revision: String,
    },
    /// Review a Git two-dot or three-dot revision range.
    Range {
        #[arg(value_parser = revision_range)]
        range: String,
    },
}

impl Args {
    pub fn request(&self) -> DiffRequest {
        DiffRequest {
            source: self.diff_source(),
            context_lines: self.context,
        }
    }

    fn diff_source(&self) -> DiffSource {
        match (&self.source, &self.shortcut) {
            (Some(SourceCommand::Git { comparison }), None) => {
                DiffSource::Git(comparison.clone().into())
            }
            (Some(SourceCommand::Patch { input }), None) => patch_input(input),
            (None, Some(input)) if input == "-" => DiffSource::Patch(PatchInput::Stdin),
            (None, Some(input)) if is_revision_range(input) => {
                DiffSource::Git(GitComparison::Range(input.clone()))
            }
            (None, Some(input)) => DiffSource::Git(GitComparison::Revision(input.clone())),
            (None, None) => DiffSource::Git(GitComparison::Changes),
            (Some(_), Some(_)) => unreachable!("clap rejects a shortcut combined with a source"),
        }
    }
}

impl From<GitCommand> for GitComparison {
    fn from(value: GitCommand) -> Self {
        match value {
            GitCommand::Changes => Self::Changes,
            GitCommand::Staged => Self::Staged,
            GitCommand::Unstaged => Self::Unstaged,
            GitCommand::Revision { revision } => Self::Revision(revision),
            GitCommand::Range { range } => Self::Range(range),
        }
    }
}

impl From<&GitCommand> for GitComparison {
    fn from(value: &GitCommand) -> Self {
        value.clone().into()
    }
}

impl Clone for GitCommand {
    fn clone(&self) -> Self {
        match self {
            Self::Changes => Self::Changes,
            Self::Staged => Self::Staged,
            Self::Unstaged => Self::Unstaged,
            Self::Revision { revision } => Self::Revision {
                revision: revision.clone(),
            },
            Self::Range { range } => Self::Range {
                range: range.clone(),
            },
        }
    }
}

fn patch_input(input: &str) -> DiffSource {
    if input == "-" {
        DiffSource::Patch(PatchInput::Stdin)
    } else {
        DiffSource::Patch(PatchInput::File(input.into()))
    }
}

fn revision_range(value: &str) -> Result<String, String> {
    split_revision_range(value)
        .is_some()
        .then(|| value.to_owned())
        .ok_or_else(|| {
        "expected a Git revision range such as A..B or A...B; use `git revision <REVISION>` for one revision".into()
    })
}

fn revision_input(value: &str) -> Result<String, String> {
    if value.contains("..") {
        return Err(
            "expected one Git revision; use `git range <A..B|A...B>` for a revision range".into(),
        );
    }
    Ok(value.into())
}

fn shortcut_input(value: &str) -> Result<String, String> {
    if value.contains("..") && split_revision_range(value).is_none() {
        return Err(
            "malformed revision range; use A..B or A...B, or `git revision <REVISION>` for one revision"
                .into(),
        );
    }
    Ok(value.into())
}

fn is_revision_range(value: &str) -> bool {
    split_revision_range(value).is_some()
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Args;
    use crate::domain::diff::{DiffSource, GitComparison, PatchInput};

    fn source(arguments: &[&str]) -> DiffSource {
        Args::try_parse_from(arguments).unwrap().request().source
    }

    #[test]
    fn canonical_forms_normalize_to_domain_sources() {
        assert_eq!(source(&["revia"]), DiffSource::Git(GitComparison::Changes));
        assert_eq!(
            source(&["revia", "git", "changes"]),
            DiffSource::Git(GitComparison::Changes)
        );
        assert_eq!(
            source(&["revia", "git", "staged"]),
            DiffSource::Git(GitComparison::Staged)
        );
        assert_eq!(
            source(&["revia", "git", "unstaged"]),
            DiffSource::Git(GitComparison::Unstaged)
        );
        assert_eq!(
            source(&["revia", "git", "revision", "HEAD~1"]),
            DiffSource::Git(GitComparison::Revision("HEAD~1".into()))
        );
        assert_eq!(
            source(&["revia", "git", "range", "main...HEAD"]),
            DiffSource::Git(GitComparison::Range("main...HEAD".into()))
        );
        assert_eq!(
            source(&["revia", "patch", "review.patch"]),
            DiffSource::Patch(PatchInput::File("review.patch".into()))
        );
    }

    #[test]
    fn global_options_apply_to_canonical_subcommands() {
        let args =
            Args::try_parse_from(["revia", "--print", "--context", "7", "git", "changes"]).unwrap();

        assert!(args.print);
        assert_eq!(args.context, 7);
        assert_eq!(
            args.request().source,
            DiffSource::Git(GitComparison::Changes)
        );
    }

    #[test]
    fn shortcuts_match_their_canonical_forms() {
        assert_eq!(
            source(&["revia", "HEAD"]),
            source(&["revia", "git", "revision", "HEAD"])
        );
        assert_eq!(
            source(&["revia", "main..HEAD"]),
            source(&["revia", "git", "range", "main..HEAD"])
        );
        assert_eq!(
            source(&["revia", "main...HEAD"]),
            source(&["revia", "git", "range", "main...HEAD"])
        );
        assert_eq!(source(&["revia", "-"]), source(&["revia", "patch", "-"]));
    }

    #[test]
    fn invalid_range_explains_the_canonical_resolution() {
        let error = Args::try_parse_from(["revia", "git", "range", "HEAD"]).unwrap_err();
        assert!(error.to_string().contains("use `git revision <REVISION>`"));
    }

    #[test]
    fn malformed_range_shortcut_explains_the_resolution() {
        let error = Args::try_parse_from(["revia", "main.."]).unwrap_err();
        assert!(error.to_string().contains("use A..B or A...B"));
    }

    #[test]
    fn canonical_commands_reject_the_wrong_revision_shape() {
        let revision =
            Args::try_parse_from(["revia", "git", "revision", "main..HEAD"]).unwrap_err();
        assert!(revision.to_string().contains("use `git range"));

        let range = Args::try_parse_from(["revia", "git", "range", "main....HEAD"]).unwrap_err();
        assert!(range.to_string().contains("use `git revision"));
    }

    #[test]
    fn source_and_shortcut_conflict_is_rejected() {
        let error = Args::try_parse_from(["revia", "git", "changes", "HEAD"]).unwrap_err();
        assert!(error.to_string().contains("unexpected argument 'HEAD'"));
    }
}
