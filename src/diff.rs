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
    pub document: DiffDocument,
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

        let text = String::from_utf8(output.stdout).context("git produced a non-UTF-8 diff")?;
        Ok(Self {
            document: DiffDocument::parse(&text),
            text,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffDocument {
    pub files: Vec<DiffFile>,
}

impl DiffDocument {
    pub fn parse(text: &str) -> Self {
        let mut files = Vec::new();
        let mut current: Option<DiffFile> = None;

        for line in text.lines() {
            if line.starts_with("diff --git ") {
                if let Some(file) = current.take() {
                    files.push(file);
                }
                current = Some(DiffFile::from_header(line));
                continue;
            }

            let Some(file) = current.as_mut() else {
                continue;
            };

            if line.starts_with("@@") {
                file.hunks.push(DiffHunk {
                    header: line.to_owned(),
                    lines: Vec::new(),
                });
            } else if let Some(hunk) = file.hunks.last_mut() {
                hunk.lines.push(DiffLine::from_raw(line));
            } else {
                file.absorb_metadata(line);
            }
        }

        if let Some(file) = current {
            files.push(file);
        }

        Self { files }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffFile {
    pub path: String,
    pub previous_path: Option<String>,
    pub metadata: Vec<String>,
    pub hunks: Vec<DiffHunk>,
}

impl DiffFile {
    fn from_header(header: &str) -> Self {
        let paths = header
            .strip_prefix("diff --git ")
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>();
        let path = paths
            .get(1)
            .or_else(|| paths.first())
            .map(|path| strip_git_prefix(path))
            .unwrap_or_else(|| "(unknown file)".to_owned());
        let previous_path = paths.first().map(|path| strip_git_prefix(path));

        Self {
            path,
            previous_path,
            metadata: vec![header.to_owned()],
            hunks: Vec::new(),
        }
    }

    fn absorb_metadata(&mut self, line: &str) {
        if let Some(path) = line.strip_prefix("+++ ") {
            self.path = normalize_patch_path(path);
        } else if let Some(path) = line.strip_prefix("--- ") {
            self.previous_path = Some(normalize_patch_path(path));
        }
        self.metadata.push(line.to_owned());
    }

    pub fn extension(&self) -> Option<&str> {
        Path::new(&self.path)
            .extension()
            .and_then(|extension| extension.to_str())
    }
}

fn strip_git_prefix(path: &str) -> String {
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
        .to_owned()
}

fn normalize_patch_path(path: &str) -> String {
    if path == "/dev/null" {
        return path.to_owned();
    }
    strip_git_prefix(path.split('\t').next().unwrap_or(path))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffHunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
}

impl DiffLine {
    fn from_raw(line: &str) -> Self {
        let (kind, text) = match line.as_bytes().first() {
            Some(b'+') => (DiffLineKind::Added, &line[1..]),
            Some(b'-') => (DiffLineKind::Removed, &line[1..]),
            Some(b' ') => (DiffLineKind::Context, &line[1..]),
            _ => (DiffLineKind::Meta, line),
        };
        Self {
            kind,
            text: text.to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Added,
    Removed,
    Context,
    Meta,
}

#[cfg(test)]
mod tests {
    use super::{DiffDocument, DiffLineKind, DiffRequest, DiffTarget};

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

    #[test]
    fn parses_files_hunks_and_line_kinds() {
        let document = DiffDocument::parse(
            "diff --git a/src/lib.rs b/src/lib.rs\nindex 111..222 100644\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1,2 @@\n old\n+new\n\\ No newline at end of file\ndiff --git a/readme.md b/readme.md\nnew file mode 100644\n--- /dev/null\n+++ b/readme.md\n@@ -0,0 +1 @@\n+hello\n",
        );

        assert_eq!(document.files.len(), 2);
        assert_eq!(document.files[0].path, "src/lib.rs");
        assert_eq!(document.files[0].hunks.len(), 1);
        assert_eq!(
            document.files[0].hunks[0].lines[1].kind,
            DiffLineKind::Added
        );
        assert_eq!(document.files[1].path, "readme.md");
    }
}
