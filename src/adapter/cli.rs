//! Command-line transport for choosing a Git diff.
//!
//! This module translates Clap's external argument representation into the
//! domain `DiffRequest`; it does not load Git data or start the terminal.

use std::path::PathBuf;

use clap::{ArgGroup, Parser};

use crate::domain::diff::{DiffRequest, DiffTarget};

/// Review Git diffs in the terminal without changing the repository.
#[derive(Debug, Parser)]
#[command(version, about)]
#[command(group(
    ArgGroup::new("target")
        .args(["staged", "commit", "range"])
        .multiple(false)
))]
pub struct Args {
    /// Git repository to inspect (defaults to the current directory).
    #[arg(short, long, default_value = ".")]
    pub repo: PathBuf,

    /// Show the index diff instead of the working-tree diff.
    #[arg(short, long)]
    pub staged: bool,

    /// Show the patch introduced by one commit or revision.
    #[arg(short, long)]
    pub commit: Option<String>,

    /// Show a Git revision range, for example main...HEAD.
    #[arg(short = 'R', long)]
    pub range: Option<String>,

    /// Number of unchanged lines surrounding each hunk.
    #[arg(short = 'U', long, default_value_t = 3)]
    pub context: usize,

    /// Write the raw Git diff to stdout instead of opening the TUI.
    #[arg(long)]
    pub print: bool,
}

impl Args {
    pub fn request(&self) -> DiffRequest {
        DiffRequest {
            target: self.target(),
            context_lines: self.context,
        }
    }

    fn target(&self) -> DiffTarget {
        if self.staged {
            DiffTarget::Staged
        } else if let Some(commit) = &self.commit {
            DiffTarget::Commit(commit.clone())
        } else if let Some(range) = &self.range {
            DiffTarget::Range(range.clone())
        } else {
            DiffTarget::WorkingTree
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Args;
    use crate::domain::diff::DiffTarget;

    #[test]
    fn defaults_to_the_working_tree() {
        let args = Args {
            repo: ".".into(),
            staged: false,
            commit: None,
            range: None,
            context: 3,
            print: false,
        };
        assert_eq!(args.request().target, DiffTarget::WorkingTree);
    }
}
