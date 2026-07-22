use std::{path::Path, process::Command};

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffTarget {
    WorkingTree,
    Staged,
    Commit(String),
    Range(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffRequest {
    pub target: DiffTarget,
    pub context_lines: usize,
}

impl DiffRequest {
    pub fn git_arguments(&self) -> Vec<String> {
        let mut arguments = vec!["diff".into(), "--no-ext-diff".into()];
        arguments.push(format!("--unified={}", self.context_lines));

        match &self.target {
            DiffTarget::WorkingTree => {}
            DiffTarget::Staged => arguments.push("--cached".into()),
            DiffTarget::Commit(revision) => {
                arguments[0] = "show".into();
                arguments.extend(["--format=".into(), revision.clone()]);
            }
            DiffTarget::Range(range) => arguments.push(range.clone()),
        }

        arguments
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedDiff {
    pub text: String,
}

impl LoadedDiff {
    pub fn load(repository: &Path, request: &DiffRequest) -> Result<Self> {
        let output = Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(request.git_arguments())
            .output()
            .with_context(|| format!("could not run git in {}", repository.display()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("git could not load this diff: {}", stderr.trim());
        }

        Ok(Self {
            text: String::from_utf8(output.stdout).context("git produced a non-UTF-8 diff")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{DiffRequest, DiffTarget};

    #[test]
    fn working_tree_uses_git_diff_with_requested_context() {
        let request = DiffRequest {
            target: DiffTarget::WorkingTree,
            context_lines: 7,
        };

        assert_eq!(
            request.git_arguments(),
            ["diff", "--no-ext-diff", "--unified=7"]
        );
    }

    #[test]
    fn staged_and_revision_targets_preserve_git_fidelity() {
        let staged = DiffRequest {
            target: DiffTarget::Staged,
            context_lines: 3,
        };
        assert_eq!(
            staged.git_arguments(),
            ["diff", "--no-ext-diff", "--unified=3", "--cached"]
        );

        let commit = DiffRequest {
            target: DiffTarget::Commit("HEAD".into()),
            context_lines: 3,
        };
        assert_eq!(
            commit.git_arguments(),
            ["show", "--no-ext-diff", "--unified=3", "--format=", "HEAD"]
        );
    }
}
