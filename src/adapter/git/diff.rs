use std::{path::Path, process::Command};

use anyhow::{Context, Result, bail};

use crate::domain::diff::{DiffDocument, DiffRequest, DiffTarget, LoadedDiff};

pub fn load(repository: &Path, request: &DiffRequest) -> Result<LoadedDiff> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(arguments(request))
        .output()
        .with_context(|| format!("could not run git in {}", repository.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "git could not load the selected {} diff: {}",
            request.target.description(),
            stderr.trim()
        );
    }

    let text = String::from_utf8(output.stdout).context("git produced a non-UTF-8 diff")?;
    Ok(LoadedDiff {
        document: DiffDocument::parse(&text),
        text,
    })
}

fn arguments(request: &DiffRequest) -> Vec<String> {
    let mut arguments = vec!["diff".into(), "--no-ext-diff".into()];
    arguments.push(format!("--unified={}", request.context_lines));

    match &request.target {
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

#[cfg(test)]
mod tests {
    use crate::domain::diff::{DiffRequest, DiffTarget};

    use super::arguments;

    #[test]
    fn working_tree_uses_git_diff_with_requested_context() {
        let request = DiffRequest {
            target: DiffTarget::WorkingTree,
            context_lines: 7,
        };

        assert_eq!(
            arguments(&request),
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
            arguments(&staged),
            ["diff", "--no-ext-diff", "--unified=3", "--cached"]
        );

        let commit = DiffRequest {
            target: DiffTarget::Commit("HEAD".into()),
            context_lines: 3,
        };
        assert_eq!(
            arguments(&commit),
            ["show", "--no-ext-diff", "--unified=3", "--format=", "HEAD"]
        );
    }
}
