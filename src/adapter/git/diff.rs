use std::{
    path::Path,
    process::{Command, Output},
};

use anyhow::{Context, Result, bail};

use crate::domain::anchor::MutableGitComparison;
use crate::domain::diff::{
    CapturedFile, CapturedFileContent, CapturedFileEvidence, CapturedFileSide, CapturedFileState,
    CapturedGitComparison, CapturedGitInput, CapturedInput, ContentId, FileStatus, GitComparison,
    ParsedPatch, ParsedPatchBody, PatchContent, decode_git_path, split_revision_range,
};

use super::changes;

pub fn capture(
    repository: &Path,
    comparison: &GitComparison,
    context_lines: usize,
) -> Result<CapturedInput> {
    let repository_root = changes::repository_root(repository)?;
    let captured = match comparison {
        GitComparison::Changes => {
            let head = resolve_commit(&repository_root, "HEAD")
                .context("mutable Git input requires a repository with HEAD")?;
            let mut text = run_diff(
                &repository_root,
                [
                    "diff".to_owned(),
                    "--no-ext-diff".to_owned(),
                    format!("--unified={context_lines}"),
                    head.clone(),
                    "--".to_owned(),
                ],
            )?;
            text.push_str(&changes::untracked_patch(&repository_root, context_lines)?);
            CapturedGitComparison::Changes {
                head,
                patch: PatchContent::new(text),
            }
        }
        GitComparison::Staged => {
            let head = resolve_commit(&repository_root, "HEAD")
                .context("mutable Git input requires a repository with HEAD")?;
            let text = run_diff(
                &repository_root,
                [
                    "diff".to_owned(),
                    "--no-ext-diff".to_owned(),
                    format!("--unified={context_lines}"),
                    "--cached".to_owned(),
                    head.clone(),
                    "--".to_owned(),
                ],
            )?;
            CapturedGitComparison::Staged {
                head,
                patch: PatchContent::new(text),
            }
        }
        GitComparison::Unstaged => {
            let head = resolve_commit(&repository_root, "HEAD")
                .context("mutable Git input requires a repository with HEAD")?;
            let text = run_diff(
                &repository_root,
                [
                    "diff".to_owned(),
                    "--no-ext-diff".to_owned(),
                    format!("--unified={context_lines}"),
                    "--".to_owned(),
                ],
            )?;
            CapturedGitComparison::Unstaged {
                head,
                patch: PatchContent::new(text),
            }
        }
        GitComparison::Revision(revision) => {
            let commit = resolve_commit(&repository_root, revision)?;
            let text = run_diff(
                &repository_root,
                [
                    "show".to_owned(),
                    "--no-ext-diff".to_owned(),
                    format!("--unified={context_lines}"),
                    "--format=".to_owned(),
                    commit.clone(),
                    "--".to_owned(),
                ],
            )?;
            CapturedGitComparison::Revision {
                requested: revision.clone(),
                commit,
                patch: PatchContent::new(text),
            }
        }
        GitComparison::Range(range) => {
            let ResolvedRange {
                left,
                right,
                base,
                target,
            } = resolve_range(&repository_root, range)?;
            let text = run_diff(
                &repository_root,
                [
                    "diff".to_owned(),
                    "--no-ext-diff".to_owned(),
                    format!("--unified={context_lines}"),
                    base.clone(),
                    target.clone(),
                    "--".to_owned(),
                ],
            )?;
            CapturedGitComparison::Range {
                requested: range.clone(),
                left,
                right,
                base,
                target,
                patch: PatchContent::new(text),
            }
        }
    };
    let parsed = ParsedPatch::parse(captured.patch())?;
    let file_evidence = capture_file_evidence(&repository_root, &captured, &parsed)?;
    if captured.immutable_target().is_none() {
        let observed_again = recapture_mutable_patch(&repository_root, &captured, context_lines)?;
        if observed_again.content_id() != captured.patch().content_id() {
            bail!("mutable Git input changed while it was being captured");
        }
    }
    Ok(CapturedInput::Git(CapturedGitInput::new(
        repository_root,
        captured,
        file_evidence,
    )))
}

#[derive(Clone)]
enum FileSource {
    MissingTree,
    Tree(String),
    Index,
    Worktree,
}

fn capture_file_evidence(
    repository: &Path,
    comparison: &CapturedGitComparison,
    parsed: &ParsedPatch,
) -> Result<Vec<CapturedFileEvidence>> {
    let (before_source, after_source) = file_sources(repository, comparison)?;
    Ok(parsed
        .files()
        .iter()
        .map(|file| {
            if matches!(file.body(), ParsedPatchBody::Combined { .. }) {
                return CapturedFileEvidence::PatchOnly;
            }
            let before_path = file.previous_path().unwrap_or(file.path());
            let before = if file.change().status == FileStatus::Added {
                Ok(CapturedFileSide::Missing)
            } else {
                capture_side(repository, &before_source, before_path)
            };
            let after = if file.change().status == FileStatus::Deleted {
                Ok(CapturedFileSide::Missing)
            } else {
                capture_side(repository, &after_source, file.path())
            };
            match (before, after) {
                (Ok(before), Ok(after)) => {
                    let state = CapturedFileState {
                        before: if file.change().binary {
                            binary_side(before)
                        } else {
                            before
                        },
                        after: if file.change().binary {
                            binary_side(after)
                        } else {
                            after
                        },
                    };
                    if state.before == state.after {
                        CapturedFileEvidence::PatchOnly
                    } else {
                        CapturedFileEvidence::Complete(state)
                    }
                }
                _ => CapturedFileEvidence::PatchOnly,
            }
        })
        .collect())
}

fn binary_side(side: CapturedFileSide) -> CapturedFileSide {
    match side {
        CapturedFileSide::Missing => CapturedFileSide::Missing,
        CapturedFileSide::Present(mut file) => {
            if let CapturedFileContent::Text(text) = &file.content {
                file.content = CapturedFileContent::Binary(ContentId::from_bytes(text.as_bytes()));
            }
            CapturedFileSide::Present(file)
        }
    }
}

fn file_sources(
    repository: &Path,
    comparison: &CapturedGitComparison,
) -> Result<(FileSource, FileSource)> {
    Ok(match comparison {
        CapturedGitComparison::Changes { head, .. } => {
            (FileSource::Tree(head.clone()), FileSource::Worktree)
        }
        CapturedGitComparison::Staged { head, .. } => {
            (FileSource::Tree(head.clone()), FileSource::Index)
        }
        CapturedGitComparison::Unstaged { .. } => (FileSource::Index, FileSource::Worktree),
        CapturedGitComparison::Revision { commit, .. } => (
            resolve_parent(repository, commit)?
                .map(FileSource::Tree)
                .unwrap_or(FileSource::MissingTree),
            FileSource::Tree(commit.clone()),
        ),
        CapturedGitComparison::Range { base, target, .. } => (
            FileSource::Tree(base.clone()),
            FileSource::Tree(target.clone()),
        ),
    })
}

fn resolve_parent(repository: &Path, commit: &str) -> Result<Option<String>> {
    let expression = format!("{commit}^");
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["rev-parse", "--verify", "--end-of-options", &expression])
        .output()
        .context("could not inspect the revision parent")?;
    if output.status.success() {
        Ok(Some(output_text(output, "parent commit identity")?))
    } else {
        Ok(None)
    }
}

fn capture_side(repository: &Path, source: &FileSource, path: &str) -> Result<CapturedFileSide> {
    if matches!(source, FileSource::MissingTree) {
        return Ok(CapturedFileSide::Missing);
    }
    let repository_path = repository_path(path)?;
    let file = match source {
        FileSource::MissingTree => return Ok(CapturedFileSide::Missing),
        FileSource::Tree(tree) => tree_file(repository, tree, path, &repository_path)?,
        FileSource::Index => index_file(repository, path, &repository_path)?,
        FileSource::Worktree => worktree_file(repository, path, &repository_path)?,
    };
    Ok(file.map_or(CapturedFileSide::Missing, CapturedFileSide::Present))
}

fn tree_file(
    repository: &Path,
    tree: &str,
    display_path: &str,
    path: &std::path::Path,
) -> Result<Option<CapturedFile>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["ls-tree", "-z", tree, "--"])
        .arg(path)
        .output()
        .context("could not inspect a Git tree file")?;
    if !output.status.success() {
        bail!("git could not inspect a tree file");
    }
    parse_git_file_record(repository, display_path, &output.stdout, false)
}

fn index_file(
    repository: &Path,
    display_path: &str,
    path: &std::path::Path,
) -> Result<Option<CapturedFile>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["ls-files", "-s", "-z", "--"])
        .arg(path)
        .output()
        .context("could not inspect an index file")?;
    if !output.status.success() {
        bail!("git could not inspect an index file");
    }
    parse_git_file_record(repository, display_path, &output.stdout, true)
}

fn parse_git_file_record(
    repository: &Path,
    path: &str,
    output: &[u8],
    index: bool,
) -> Result<Option<CapturedFile>> {
    let records = output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .collect::<Vec<_>>();
    if records.is_empty() {
        return Ok(None);
    }
    if records.len() != 1 {
        bail!("file has multiple index stages");
    }
    let header = records[0]
        .split(|byte| *byte == b'\t')
        .next()
        .and_then(|header| std::str::from_utf8(header).ok())
        .ok_or_else(|| anyhow::anyhow!("Git returned malformed file metadata"))?;
    let fields = header.split_whitespace().collect::<Vec<_>>();
    let (mode, object) = if index {
        if fields.get(2) != Some(&"0") {
            bail!("file has an unmerged index entry");
        }
        (
            *fields
                .first()
                .ok_or_else(|| anyhow::anyhow!("missing mode"))?,
            *fields
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("missing object"))?,
        )
    } else {
        (
            *fields
                .first()
                .ok_or_else(|| anyhow::anyhow!("missing mode"))?,
            *fields
                .get(2)
                .ok_or_else(|| anyhow::anyhow!("missing object"))?,
        )
    };
    let content = if mode == "160000" {
        CapturedFileContent::Gitlink(object.to_owned())
    } else {
        let output = run_git(repository, ["cat-file", "-p", object])?;
        if mode == "120000" {
            CapturedFileContent::Symlink(output.stdout.into())
        } else {
            captured_content(output.stdout)
        }
    };
    Ok(Some(CapturedFile {
        path: path.to_owned(),
        mode: mode.to_owned(),
        content,
    }))
}

fn worktree_file(
    repository: &Path,
    display_path: &str,
    path: &std::path::Path,
) -> Result<Option<CapturedFile>> {
    let full_path = repository.join(path);
    let Ok(metadata) = std::fs::symlink_metadata(&full_path) else {
        return Ok(None);
    };
    if metadata.is_dir() {
        let object = output_text(
            run_git(&full_path, ["rev-parse", "--verify", "HEAD"])?,
            "submodule identity",
        )?;
        return Ok(Some(CapturedFile {
            path: display_path.to_owned(),
            mode: "160000".into(),
            content: CapturedFileContent::Gitlink(object),
        }));
    }
    let (mode, bytes) = if metadata.file_type().is_symlink() {
        let target = std::fs::read_link(&full_path)?;
        ("120000".to_owned(), path_bytes(&target))
    } else {
        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode() & 0o111 != 0
        };
        #[cfg(not(unix))]
        let executable = false;
        (
            if executable { "100755" } else { "100644" }.to_owned(),
            std::fs::read(&full_path)?,
        )
    };
    let content = if mode == "120000" {
        CapturedFileContent::Symlink(bytes.into())
    } else {
        captured_content(bytes)
    };
    Ok(Some(CapturedFile {
        path: display_path.to_owned(),
        mode,
        content,
    }))
}

fn captured_content(bytes: Vec<u8>) -> CapturedFileContent {
    if bytes.contains(&0) {
        return CapturedFileContent::Binary(ContentId::from_bytes(&bytes));
    }
    match String::from_utf8(bytes) {
        Ok(text) => CapturedFileContent::Text(text.into()),
        Err(error) => CapturedFileContent::Binary(ContentId::from_bytes(error.as_bytes())),
    }
}

fn repository_path(path: &str) -> Result<std::path::PathBuf> {
    if let Some(bytes) = decode_git_path(path) {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            return Ok(std::ffi::OsString::from_vec(bytes).into());
        }
        #[cfg(not(unix))]
        return String::from_utf8(bytes)
            .map(Into::into)
            .context("Git path is not valid UTF-8 on this platform");
    }
    Ok(path.into())
}

#[cfg(unix)]
fn path_bytes(path: &std::path::Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
fn path_bytes(path: &std::path::Path) -> Vec<u8> {
    path.to_string_lossy().as_bytes().to_vec()
}

fn recapture_mutable_patch(
    repository: &Path,
    comparison: &CapturedGitComparison,
    context_lines: usize,
) -> Result<PatchContent> {
    let mut text = match comparison {
        CapturedGitComparison::Changes { head, .. } => run_diff(
            repository,
            [
                "diff".to_owned(),
                "--no-ext-diff".to_owned(),
                format!("--unified={context_lines}"),
                head.clone(),
                "--".to_owned(),
            ],
        )?,
        CapturedGitComparison::Staged { head, .. } => run_diff(
            repository,
            [
                "diff".to_owned(),
                "--no-ext-diff".to_owned(),
                format!("--unified={context_lines}"),
                "--cached".to_owned(),
                head.clone(),
                "--".to_owned(),
            ],
        )?,
        CapturedGitComparison::Unstaged { .. } => run_diff(
            repository,
            [
                "diff".to_owned(),
                "--no-ext-diff".to_owned(),
                format!("--unified={context_lines}"),
                "--".to_owned(),
            ],
        )?,
        CapturedGitComparison::Revision { .. } | CapturedGitComparison::Range { .. } => {
            bail!("immutable comparison does not require recapture")
        }
    };
    if matches!(comparison, CapturedGitComparison::Changes { .. }) {
        text.push_str(&changes::untracked_patch(repository, context_lines)?);
    }
    Ok(PatchContent::new(text))
}

pub fn snapshot_patch(
    repository: &Path,
    comparison: &MutableGitComparison,
    object: &str,
    context_lines: usize,
) -> Result<PatchContent> {
    let repository = changes::repository_root(repository)?;
    let text = match comparison {
        MutableGitComparison::Changes { head } | MutableGitComparison::Staged { head } => run_diff(
            &repository,
            [
                "diff".to_owned(),
                "--no-ext-diff".to_owned(),
                format!("--unified={context_lines}"),
                head.clone(),
                object.to_owned(),
                "--".to_owned(),
            ],
        )?,
        MutableGitComparison::Unstaged => run_diff(
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
    };
    Ok(PatchContent::new(text))
}

fn resolve_commit(repository: &Path, revision: &str) -> Result<String> {
    let expression = format!("{revision}^{{commit}}");
    let output = run_git(
        repository,
        ["rev-parse", "--verify", "--end-of-options", &expression],
    )?;
    output_text(output, "commit identity")
}

struct ResolvedRange {
    left: String,
    right: String,
    base: String,
    target: String,
}

fn resolve_range(repository: &Path, range: &str) -> Result<ResolvedRange> {
    let (left, notation, right) = split_revision_range(range)
        .ok_or_else(|| anyhow::anyhow!("expected exactly one A..B or A...B revision range"))?;
    let left = resolve_commit(repository, left)?;
    let right = resolve_commit(repository, right)?;
    let base = if notation == "..." {
        output_text(
            run_git(repository, ["merge-base", left.as_str(), right.as_str()])?,
            "merge-base identity",
        )?
    } else {
        left.clone()
    };
    Ok(ResolvedRange {
        left,
        right: right.clone(),
        base,
        target: right,
    })
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

    use super::capture;
    use crate::domain::anchor::MutableGitComparison;
    use crate::domain::diff::{
        CapturedGitComparison, CapturedInput, CompleteFileDiff, FileDiff, FileStatus,
        GitComparison, ReviewPresentation, build_review,
    };

    fn presentation(captured: &CapturedInput) -> std::sync::Arc<ReviewPresentation> {
        build_review(captured, 3).unwrap().1
    }

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

        let loaded = capture(repository.path(), &GitComparison::Changes, 3).unwrap();
        let shown = presentation(&loaded);
        let paths = shown
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>();

        assert_eq!(paths, ["staged.txt", "unstaged.txt", "untracked.txt"]);
        assert!(loaded.patch().text().contains("+after staged"));
        assert!(loaded.patch().text().contains("+after unstaged"));
        assert!(loaded.patch().text().contains("+new file"));
        assert!(
            build_review(&loaded, 3)
                .unwrap()
                .0
                .files()
                .iter()
                .all(|file| matches!(file, FileDiff::Complete(_)))
        );
    }

    #[test]
    fn changes_includes_empty_untracked_files() {
        let repository = Repository::new();
        repository.write("tracked.txt", "base\n");
        repository.git(["add", "."]);
        repository.git(["commit", "-qm", "base"]);
        repository.write("empty.txt", "");

        let loaded = capture(repository.path(), &GitComparison::Changes, 3).unwrap();

        let shown = presentation(&loaded);
        assert_eq!(shown.files.len(), 1);
        assert_eq!(shown.files[0].path, "empty.txt");
        assert_eq!(shown.files[0].change.status, FileStatus::Added);
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

        let loaded = capture(repository.path(), &GitComparison::Changes, 3).unwrap();

        let shown = presentation(&loaded);
        assert_eq!(shown.files.len(), 1);
        assert!(loaded.patch().text().contains("invalid-\\200.txt"));
        let path = shown.files[0].path.clone();
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

        let loaded = capture(
            &repository.path().join("nested"),
            &GitComparison::Changes,
            3,
        )
        .unwrap();
        let shown = presentation(&loaded);
        let paths = shown
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>();

        assert_eq!(paths, ["outside.txt", "nested/untracked.txt"]);
    }

    #[test]
    fn unborn_mutable_comparisons_are_unsupported() {
        let repository = Repository::new();
        repository.write("example.txt", "staged\n");
        repository.git(["add", "example.txt"]);
        repository.write("example.txt", "working\n");

        let error = capture(repository.path(), &GitComparison::Changes, 3).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("requires a repository with HEAD")
        );
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

        let result = capture(repository.path(), &GitComparison::Range(range), 3);

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

        let revision = capture(
            repository.path(),
            &GitComparison::Revision("HEAD".into()),
            3,
        )
        .unwrap();
        assert!(revision.patch().text().contains("-base"));
        assert!(revision.patch().text().contains("+next"));
        for notation in [format!("{base}..HEAD"), format!("{base}...HEAD")] {
            let range = capture(repository.path(), &GitComparison::Range(notation), 3).unwrap();
            assert!(range.patch().text().contains("-base"));
            assert!(range.patch().text().contains("+next"));
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

        let staged = capture(repository.path(), &GitComparison::Staged, 3).unwrap();
        let unstaged = capture(repository.path(), &GitComparison::Unstaged, 3).unwrap();

        assert!(staged.patch().text().contains("conflict.txt"));
        assert!(unstaged.patch().text().contains("conflict.txt"));
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

        let changes = capture(repository.path(), &GitComparison::Changes, 3).unwrap();
        let unstaged = capture(repository.path(), &GitComparison::Unstaged, 3).unwrap();

        assert!(changes.patch().text().contains("-dirty"));
        assert!(unstaged.patch().text().contains("-dirty"));
        assert!(build_review(&changes, 3).is_ok());
        assert!(build_review(&unstaged, 3).is_ok());
        let anchors = crate::adapter::git::anchor::AnchorStore::new(repository.path());
        let object = anchors.snapshot_changes().unwrap();
        let comparison = match changes.git_comparison().unwrap() {
            CapturedGitComparison::Changes { head, .. } => {
                MutableGitComparison::Changes { head: head.clone() }
            }
            _ => unreachable!("changes capture has changes comparison"),
        };
        let snapshotted =
            super::snapshot_patch(repository.path(), &comparison, &object, 3).unwrap();
        assert_ne!(changes.content_id(), snapshotted.content_id());
        assert!(anchors.snapshot_working_tree().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn regular_file_to_symlink_is_a_complete_replacement() {
        let repository = Repository::new();
        repository.write("example", "regular\n");
        repository.git(["add", "."]);
        repository.git(["commit", "-qm", "base"]);
        fs::remove_file(repository.path().join("example")).unwrap();
        std::os::unix::fs::symlink("target", repository.path().join("example")).unwrap();

        let captured = capture(repository.path(), &GitComparison::Changes, 3).unwrap();
        let diff = build_review(&captured, 3).unwrap().0;

        assert!(
            matches!(
                diff.files(),
                [FileDiff::Complete(CompleteFileDiff::Replacement(_))]
            ),
            "{:#?}",
            diff.files()
        );
    }
}
