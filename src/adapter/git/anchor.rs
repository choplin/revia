use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};

use crate::domain::anchor::{Anchor, HunkLocation};

static TEMPORARY_INDEX_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub struct AnchorStore<'a> {
    repository: &'a Path,
}

impl<'a> AnchorStore<'a> {
    pub fn new(repository: &'a Path) -> Self {
        Self { repository }
    }

    pub fn object(
        &self,
        object: &str,
        path: impl Into<String>,
        hunk_header: impl Into<String>,
    ) -> Result<Anchor> {
        let object = self.git([
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{object}^{{object}}"),
        ])?;
        let object = object.trim();
        self.protect(object)?;
        Ok(Anchor::new(object, HunkLocation::new(path, hunk_header)))
    }

    /// Returns a candidate complete index-plus-worktree tree. The caller must
    /// verify its reconstructed comparison before caching or anchoring it.
    pub fn snapshot_changes(&self) -> Result<String> {
        let index = TemporaryIndex::new()?;
        let repository_index =
            self.git(["rev-parse", "--path-format=absolute", "--git-path", "index"])?;
        let repository_index = PathBuf::from(repository_index.trim());
        if repository_index.exists() {
            fs::copy(&repository_index, index.path()).with_context(|| {
                format!(
                    "could not copy repository index {}",
                    repository_index.display()
                )
            })?;
        } else {
            self.git_with_index(index.path(), ["read-tree", "--empty"])?;
        }
        self.git_with_index(index.path(), ["add", "-A", "--", ":/"])?;
        Ok(self
            .git_with_index(index.path(), ["write-tree"])?
            .trim()
            .into())
    }

    pub fn snapshot_index(&self) -> Result<String> {
        Ok(self.git(["write-tree"])?.trim().into())
    }

    pub fn snapshot_working_tree(&self) -> Result<String> {
        let snapshot = self.git(["stash", "create"])?;
        let snapshot = snapshot.trim();
        if snapshot.is_empty() {
            bail!("the working tree has no tracked changes to snapshot");
        }
        Ok(snapshot.into())
    }

    pub fn resolve_file(&self, anchor: &Anchor) -> Result<String> {
        let location = anchor.location();
        let spec = object_path(
            anchor.object_id(),
            location.path(),
            anchor.decoded_path_bytes(),
        )?;
        let output = Command::new("git")
            .arg("-C")
            .arg(self.repository)
            .arg("show")
            .arg(spec)
            .output()
            .with_context(|| format!("could not run git in {}", self.repository.display()))?;
        if !output.status.success() {
            bail!(
                "git command failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        String::from_utf8(output.stdout).context("git produced non-UTF-8 output")
    }

    fn git<const N: usize>(&self, arguments: [&str; N]) -> Result<String> {
        self.command(arguments, None)
    }

    fn git_with_index<const N: usize>(&self, index: &Path, arguments: [&str; N]) -> Result<String> {
        self.command(arguments, Some(index))
    }

    fn command<const N: usize>(
        &self,
        arguments: [&str; N],
        index: Option<&Path>,
    ) -> Result<String> {
        let mut command = Command::new("git");
        command.arg("-C").arg(self.repository).args(arguments);
        if let Some(index) = index {
            command.env("GIT_INDEX_FILE", index);
        }
        let output = command
            .output()
            .with_context(|| format!("could not run git in {}", self.repository.display()))?;
        if !output.status.success() {
            bail!(
                "git command failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        String::from_utf8(output.stdout).context("git produced non-UTF-8 output")
    }

    fn protect(&self, object: &str) -> Result<()> {
        let reference = format!("refs/revia/snapshots/{}-{object}", nonce()?);
        self.git(["update-ref", &reference, object])?;
        Ok(())
    }
}

#[cfg(unix)]
fn object_path(object: &str, path: &str, decoded: Option<Vec<u8>>) -> Result<std::ffi::OsString> {
    use std::os::unix::ffi::OsStringExt;

    let mut bytes = object.as_bytes().to_vec();
    bytes.push(b':');
    bytes.extend(decoded.unwrap_or_else(|| path.as_bytes().to_vec()));
    Ok(std::ffi::OsString::from_vec(bytes))
}

#[cfg(not(unix))]
fn object_path(object: &str, path: &str, decoded: Option<Vec<u8>>) -> Result<std::ffi::OsString> {
    if decoded.is_some() {
        bail!("non-UTF-8 Git paths are unsupported on this platform");
    }
    Ok(format!("{object}:{path}").into())
}

struct TemporaryIndex {
    directory: PathBuf,
    path: PathBuf,
}

impl TemporaryIndex {
    fn new() -> Result<Self> {
        for _ in 0..100 {
            let nonce = nonce()?;
            let sequence = TEMPORARY_INDEX_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let directory = std::env::temp_dir().join(format!(
                "revia-index-{}-{nonce}-{sequence}",
                std::process::id()
            ));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&directory) {
                Ok(()) => {
                    let path = directory.join("index");
                    return Ok(Self { directory, path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error).context("could not create a private index"),
            }
        }
        bail!("could not allocate a unique temporary index")
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryIndex {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_file(self.path.with_extension("lock"));
        let _ = fs::remove_dir(&self.directory);
    }
}

fn nonce() -> Result<u128> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_nanos())
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path, process::Command};

    use super::AnchorStore;
    use crate::test_support::temp_dir;

    fn git<const N: usize>(repository: &Path, arguments: [&str; N]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    #[test]
    fn materialized_changes_anchor_exact_untracked_content_without_touching_the_index() {
        let temporary = temp_dir("anchor");
        let repository = temporary.path();
        git(repository, ["init", "-q"]);
        git(repository, ["config", "user.name", "Revia Test"]);
        git(
            repository,
            ["config", "user.email", "revia@example.invalid"],
        );
        fs::write(repository.join("tracked.rs"), "fn old() {}\n").unwrap();
        git(repository, ["add", "."]);
        git(repository, ["commit", "-qm", "base"]);
        fs::write(repository.join("untracked.rs"), "fn new_file() {}\n").unwrap();
        let index_before = git(repository, ["diff", "--cached"]);
        let store = AnchorStore::new(repository);

        let object = store.snapshot_changes().unwrap();
        let anchor = store
            .object(&object, "untracked.rs", "@@ -0,0 +1 @@")
            .unwrap();

        assert_eq!(store.resolve_file(&anchor).unwrap(), "fn new_file() {}\n");
        assert_eq!(git(repository, ["diff", "--cached"]), index_before);
    }

    #[test]
    fn legacy_marker_like_utf8_path_still_resolves_as_a_literal_filename() {
        let temporary = temp_dir("anchor");
        let repository = temporary.path();
        git(repository, ["init", "-q"]);
        git(repository, ["config", "user.name", "Revia Test"]);
        git(
            repository,
            ["config", "user.email", "revia@example.invalid"],
        );
        fs::write(repository.join("git-path:utf8:foo"), "legacy\n").unwrap();
        git(repository, ["add", "."]);
        git(repository, ["commit", "-qm", "base"]);
        let head = git(repository, ["rev-parse", "HEAD"]);
        let json = format!(
            r#"{{"revision":"{}","path":"git-path:utf8:foo","hunk_header":"@@ -0,0 +1 @@"}}"#,
            head.trim()
        );
        let anchor: crate::domain::anchor::Anchor = serde_json::from_str(&json).unwrap();

        assert_eq!(
            AnchorStore::new(repository).resolve_file(&anchor).unwrap(),
            "legacy\n"
        );
    }
}
