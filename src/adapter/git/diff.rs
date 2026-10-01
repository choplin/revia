use std::{
    path::Path,
    process::{Command, Output},
    sync::Arc,
};

use anyhow::{Context, Result, bail};

use crate::domain::diff::{
    DiffDocument, DiffProvenance, GitComparison, LoadedDiff, split_revision_range,
};

use super::changes;

pub fn load(
    repository: &Path,
    comparison: &GitComparison,
    context_lines: usize,
) -> Result<LoadedDiff> {
    let repository_root = changes::repository_root(repository)?;
    let (mut text, immutable_object) = match comparison {
        GitComparison::Changes => {
            let base = resolve_head_tree(&repository_root)?.unwrap_or_else(empty_tree_id);
            (
                run_diff(
                    &repository_root,
                    [
                        "diff".to_owned(),
                        "--no-ext-diff".to_owned(),
                        format!("--unified={context_lines}"),
                        base,
                        "--".to_owned(),
                    ],
                )?,
                None,
            )
        }
        GitComparison::Staged => (
            run_diff(
                &repository_root,
                [
                    "diff".to_owned(),
                    "--no-ext-diff".to_owned(),
                    format!("--unified={context_lines}"),
                    "--cached".to_owned(),
                    "--".to_owned(),
                ],
            )?,
            None,
        ),
        GitComparison::Unstaged => (
            run_diff(
                &repository_root,
                [
                    "diff".to_owned(),
                    "--no-ext-diff".to_owned(),
                    format!("--unified={context_lines}"),
                    "--".to_owned(),
                ],
            )?,
            None,
        ),
        GitComparison::Revision(revision) => {
            let revision = resolve_commit(&repository_root, revision)?;
            (
                run_diff(
                    &repository_root,
                    [
                        "show".to_owned(),
                        "--no-ext-diff".to_owned(),
                        format!("--unified={context_lines}"),
                        "--format=".to_owned(),
                        revision.clone(),
                        "--".to_owned(),
                    ],
                )?,
                Some(revision),
            )
        }
        GitComparison::Range(range) => {
            let (range, target) = resolve_range(&repository_root, range)?;
            (
                run_diff(
                    &repository_root,
                    [
                        "diff".to_owned(),
                        "--no-ext-diff".to_owned(),
                        format!("--unified={context_lines}"),
                        range,
                        "--".to_owned(),
                    ],
                )?,
                Some(target),
            )
        }
    };
    if matches!(comparison, GitComparison::Changes) {
        text.push_str(&changes::untracked_patch(&repository_root, context_lines)?);
    }
    let text: Arc<str> = text.into();
    let document = Arc::new(DiffDocument::parse(&text));
    let provenance = immutable_object.map_or_else(
        || DiffProvenance::MutableGit {
            expected_document: document.clone(),
            context_lines,
        },
        DiffProvenance::GitObject,
    );
    Ok(LoadedDiff {
        document,
        text,
        provenance,
    })
}

pub fn snapshot_document(
    repository: &Path,
    comparison: &GitComparison,
    object: &str,
    context_lines: usize,
) -> Result<DiffDocument> {
    let repository = changes::repository_root(repository)?;
    let text = match comparison {
        GitComparison::Changes | GitComparison::Staged => {
            let base = resolve_head_tree(&repository)?.unwrap_or_else(empty_tree_id);
            run_diff(
                &repository,
                [
                    "diff".to_owned(),
                    "--no-ext-diff".to_owned(),
                    format!("--unified={context_lines}"),
                    base,
                    object.to_owned(),
                    "--".to_owned(),
                ],
            )?
        }
        GitComparison::Unstaged => run_diff(
            &repository,
            [
                "diff".to_owned(),
                "--no-ext-diff".to_owned(),
                format!("--unified={context_lines}"),
                format!("{object}^2"),
                object.to_owned(),
                "--".to_owned(),
            ],
        )?,
        GitComparison::Revision(_) | GitComparison::Range(_) => {
            bail!("immutable comparisons do not use mutable snapshot verification")
        }
    };
    Ok(DiffDocument::parse(&text))
}

fn resolve_head_tree(repository: &Path) -> Result<Option<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["rev-parse", "--verify", "--end-of-options", "HEAD^{tree}"])
        .output()
        .context("could not inspect HEAD")?;
    if output.status.success() {
        Ok(Some(output_text(output, "tree identity")?))
    } else {
        Ok(None)
    }
}

fn empty_tree_id() -> String {
    "4b825dc642cb6eb9a060e54bf8d69288fbee4904".into()
}

fn resolve_commit(repository: &Path, revision: &str) -> Result<String> {
    let expression = format!("{revision}^{{commit}}");
    let output = run_git(
        repository,
        ["rev-parse", "--verify", "--end-of-options", &expression],
    )?;
    output_text(output, "commit identity")
}

fn resolve_range(repository: &Path, range: &str) -> Result<(String, String)> {
    let (left, notation, right) = split_revision_range(range)
        .ok_or_else(|| anyhow::anyhow!("expected exactly one A..B or A...B revision range"))?;
    let left = resolve_commit(repository, left)?;
    let right = resolve_commit(repository, right)?;
    Ok((format!("{left}{notation}{right}"), right))
}

fn run_diff<I, S>(repository: &Path, arguments: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let output = run_git(repository, arguments)?;
    String::from_utf8(output.stdout).context("git produced a non-UTF-8 diff")
}

fn run_git<I, S>(repository: &Path, arguments: I) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(arguments)
        .output()
        .with_context(|| format!("could not run git in {}", repository.display()))?;
    if !output.status.success() {
        bail!(
            "git could not load the selected comparison: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output)
}

fn output_text(output: Output, kind: &str) -> Result<String> {
    Ok(String::from_utf8(output.stdout)
        .with_context(|| format!("git produced a non-UTF-8 {kind}"))?
        .trim()
        .to_owned())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::load;
    use crate::domain::diff::{DiffProvenance, FileStatus, GitComparison};

    static REPOSITORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct Repository {
        path: PathBuf,
    }

    impl Repository {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let sequence = REPOSITORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("revia-diff-{nonce}-{sequence}"));
            fs::create_dir(&path).unwrap();
            let repository = Self { path };
            repository.git(["init", "-q"]);
            repository.git(["config", "user.name", "Revia Test"]);
            repository.git(["config", "user.email", "revia@example.invalid"]);
            repository
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn write(&self, path: &str, body: &str) {
            fs::write(self.path.join(path), body).unwrap();
        }

        fn git<const N: usize>(&self, arguments: [&str; N]) -> String {
            let output = Command::new("git")
                .arg("-C")
                .arg(&self.path)
                .args(arguments)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        }
    }

    impl Drop for Repository {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.path).unwrap();
        }
    }

    #[test]
    fn changes_combines_staged_unstaged_and_untracked_files() {
        let repository = Repository::new();
        repository.write("staged.txt", "before staged\n");
        repository.write("unstaged.txt", "before unstaged\n");
        repository.git(["add", "."]);
        repository.git(["commit", "-qm", "base"]);
        repository.write("staged.txt", "after staged\n");
        repository.git(["add", "staged.txt"]);
        repository.write("unstaged.txt", "after unstaged\n");
        repository.write("untracked.txt", "new file\n");

        let loaded = load(repository.path(), &GitComparison::Changes, 3).unwrap();
        let paths = loaded
            .document
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>();

        assert_eq!(paths, ["staged.txt", "unstaged.txt", "untracked.txt"]);
        assert!(loaded.text.contains("+after staged"));
        assert!(loaded.text.contains("+after unstaged"));
        assert!(loaded.text.contains("+new file"));
    }

    #[test]
    fn changes_includes_empty_untracked_files() {
        let repository = Repository::new();
        repository.write("tracked.txt", "base\n");
        repository.git(["add", "."]);
        repository.git(["commit", "-qm", "base"]);
        repository.write("empty.txt", "");

        let loaded = load(repository.path(), &GitComparison::Changes, 3).unwrap();

        assert_eq!(loaded.document.files.len(), 1);
        assert_eq!(loaded.document.files[0].path, "empty.txt");
        assert_eq!(loaded.document.files[0].change.status, FileStatus::Added);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn changes_accepts_non_utf8_untracked_paths() {
        use std::os::unix::ffi::OsStringExt;

        let repository = Repository::new();
        repository.write("tracked.txt", "base\n");
        repository.git(["add", "."]);
        repository.git(["commit", "-qm", "base"]);
        let name = std::ffi::OsString::from_vec(b"invalid-\x80.txt".to_vec());
        fs::write(repository.path().join(name), "new\n").unwrap();

        let loaded = load(repository.path(), &GitComparison::Changes, 3).unwrap();

        assert_eq!(loaded.document.files.len(), 1);
        assert!(loaded.text.contains("invalid-\\200.txt"));
        let path = loaded.document.files[0].path.clone();
        assert!(path.starts_with("git-path:"));
        let store = crate::adapter::git::anchor::AnchorStore::new(repository.path());
        let object = store.snapshot_changes().unwrap();
        let anchor = store.object(&object, path, "@@ -0,0 +1 @@").unwrap();
        assert_eq!(store.resolve_file(&anchor).unwrap(), "new\n");
    }

    #[test]
    fn changes_started_in_a_subdirectory_cover_the_whole_repository() {
        let repository = Repository::new();
        repository.write("outside.txt", "before\n");
        fs::create_dir(repository.path().join("nested")).unwrap();
        repository.write("nested/inside.txt", "before\n");
        repository.git(["add", "."]);
        repository.git(["commit", "-qm", "base"]);
        repository.write("outside.txt", "after\n");
        repository.write("nested/untracked.txt", "new\n");

        let loaded = load(
            &repository.path().join("nested"),
            &GitComparison::Changes,
            3,
        )
        .unwrap();
        let paths = loaded
            .document
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>();

        assert_eq!(paths, ["outside.txt", "nested/untracked.txt"]);
    }

    #[test]
    fn unborn_changes_use_the_complete_working_state() {
        let repository = Repository::new();
        repository.write("example.txt", "staged\n");
        repository.git(["add", "example.txt"]);
        repository.write("example.txt", "working\n");

        let loaded = load(repository.path(), &GitComparison::Changes, 3).unwrap();

        assert!(loaded.text.contains("+working"));
        assert!(!loaded.text.contains("+staged"));
        assert!(matches!(
            loaded.provenance,
            DiffProvenance::MutableGit { .. }
        ));
    }

    #[test]
    fn range_like_git_option_is_resolved_as_data_and_cannot_create_a_file() {
        let repository = Repository::new();
        repository.write("example.txt", "base\n");
        repository.git(["add", "."]);
        repository.git(["commit", "-qm", "base"]);
        let target = repository.path().with_extension("injected-output");
        let range = format!(
            "--output={}/../{}",
            repository.path().display(),
            target.file_name().unwrap().to_string_lossy()
        );

        let result = load(repository.path(), &GitComparison::Range(range), 3);

        assert!(result.is_err());
        assert!(!target.exists());
    }

    #[test]
    fn revision_and_ranges_keep_git_commit_semantics() {
        let repository = Repository::new();
        repository.write("example.txt", "base\n");
        repository.git(["add", "."]);
        repository.git(["commit", "-qm", "base"]);
        let base = repository.git(["rev-parse", "HEAD"]);
        repository.write("example.txt", "next\n");
        repository.git(["commit", "-qam", "next"]);

        let revision = load(
            repository.path(),
            &GitComparison::Revision("HEAD".into()),
            3,
        )
        .unwrap();
        assert!(revision.text.contains("-base"));
        assert!(revision.text.contains("+next"));
        for notation in [format!("{base}..HEAD"), format!("{base}...HEAD")] {
            let range = load(repository.path(), &GitComparison::Range(notation), 3).unwrap();
            assert!(range.text.contains("-base"));
            assert!(range.text.contains("+next"));
        }
    }

    #[test]
    fn staged_and_unstaged_remain_reviewable_with_unmerged_index_entries() {
        let repository = Repository::new();
        repository.write("conflict.txt", "base\n");
        repository.git(["add", "."]);
        repository.git(["commit", "-qm", "base"]);
        let base_branch = repository.git(["branch", "--show-current"]);
        repository.git(["switch", "-qc", "side"]);
        repository.write("conflict.txt", "side\n");
        repository.git(["commit", "-qam", "side"]);
        repository.git(["switch", "-q", &base_branch]);
        repository.write("conflict.txt", "main\n");
        repository.git(["commit", "-qam", "main"]);
        let merge = Command::new("git")
            .arg("-C")
            .arg(repository.path())
            .args(["merge", "side"])
            .output()
            .unwrap();
        assert!(!merge.status.success());

        let staged = load(repository.path(), &GitComparison::Staged, 3).unwrap();
        let unstaged = load(repository.path(), &GitComparison::Unstaged, 3).unwrap();

        assert!(staged.text.contains("conflict.txt"));
        assert!(unstaged.text.contains("conflict.txt"));
        let anchors = crate::adapter::git::anchor::AnchorStore::new(repository.path());
        assert!(anchors.snapshot_index().is_err());
        assert!(anchors.snapshot_working_tree().is_err());
    }

    #[test]
    fn changes_and_unstaged_keep_dirty_submodule_evidence() {
        let submodule = Repository::new();
        submodule.write("inside.txt", "base\n");
        submodule.git(["add", "."]);
        submodule.git(["commit", "-qm", "base"]);
        let repository = Repository::new();
        let add = Command::new("git")
            .arg("-C")
            .arg(repository.path())
            .args(["-c", "protocol.file.allow=always", "submodule", "add"])
            .arg(submodule.path())
            .arg("sub")
            .output()
            .unwrap();
        assert!(
            add.status.success(),
            "{}",
            String::from_utf8_lossy(&add.stderr)
        );
        repository.git(["commit", "-qam", "submodule"]);
        fs::write(repository.path().join("sub/inside.txt"), "dirty\n").unwrap();

        let changes = load(repository.path(), &GitComparison::Changes, 3).unwrap();
        let unstaged = load(repository.path(), &GitComparison::Unstaged, 3).unwrap();

        assert!(changes.text.contains("-dirty"));
        assert!(unstaged.text.contains("-dirty"));
        let anchors = crate::adapter::git::anchor::AnchorStore::new(repository.path());
        let object = anchors.snapshot_changes().unwrap();
        let snapshotted =
            super::snapshot_document(repository.path(), &GitComparison::Changes, &object, 3)
                .unwrap();
        assert_ne!(changes.document.as_ref(), &snapshotted);
        assert!(anchors.snapshot_working_tree().is_err());
    }
}
