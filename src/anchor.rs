use std::{
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};

/// The visible review target within one diff: a file and one hunk header.
///
/// This deliberately excludes a revision. An `Anchor` adds immutable Git
/// provenance to a location; navigation and thread grouping use the location.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HunkLocation {
    path: String,
    hunk_header: String,
}

impl HunkLocation {
    pub fn new(path: impl Into<String>, hunk_header: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            hunk_header: hunk_header.into(),
        }
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn hunk_header(&self) -> &str {
        &self.hunk_header
    }
}

/// A stable location in a Git object. The hunk header identifies the reviewed
/// region; the revision is immutable and is the source of truth for its code.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Anchor {
    revision: String,
    path: String,
    hunk_header: String,
}

impl Anchor {
    pub fn new(revision: impl Into<String>, location: HunkLocation) -> Self {
        Self {
            revision: revision.into(),
            path: location.path,
            hunk_header: location.hunk_header,
        }
    }

    pub fn revision(&self) -> &str {
        &self.revision
    }

    pub fn location(&self) -> HunkLocation {
        HunkLocation::new(&self.path, &self.hunk_header)
    }

    pub fn is_at(&self, location: &HunkLocation) -> bool {
        self.path == location.path && self.hunk_header == location.hunk_header
    }
}

pub struct AnchorStore<'a> {
    repository: &'a Path,
}

impl<'a> AnchorStore<'a> {
    pub fn new(repository: &'a Path) -> Self {
        Self { repository }
    }

    pub fn committed(
        &self,
        revision: &str,
        path: impl Into<String>,
        hunk_header: impl Into<String>,
    ) -> Result<Anchor> {
        let revision = self.git(["rev-parse", "--verify", &format!("{revision}^{{commit}}")])?;
        Ok(Anchor::new(
            revision.trim(),
            HunkLocation::new(path, hunk_header),
        ))
    }

    /// Creates an immutable stash commit without touching the stash stack, then
    /// protects it from Git GC with a private revia ref.
    pub fn snapshot_working_tree(
        &self,
        path: impl Into<String>,
        hunk_header: impl Into<String>,
    ) -> Result<Anchor> {
        let snapshot = self.git(["stash", "create"])?;
        let snapshot = snapshot.trim();
        if snapshot.is_empty() {
            bail!("the working tree has no tracked changes to snapshot");
        }

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock is before Unix epoch")?
            .as_nanos();
        let reference = format!("refs/revia/snapshots/{nonce}-{snapshot}");
        self.git(["update-ref", &reference, snapshot])?;

        Ok(Anchor::new(snapshot, HunkLocation::new(path, hunk_header)))
    }

    pub fn resolve_file(&self, anchor: &Anchor) -> Result<String> {
        self.git([
            "show",
            &format!("{}:{}", anchor.revision(), anchor.location().path()),
        ])
    }

    fn git<const N: usize>(&self, arguments: [&str; N]) -> Result<String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(self.repository)
            .args(arguments)
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
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::AnchorStore;

    fn repository() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("revia-anchor-{nonce}"));
        fs::create_dir_all(&path).unwrap();
        git(&path, ["init", "-q"]);
        git(&path, ["config", "user.name", "Revia Test"]);
        git(
            &path,
            ["config", "user.email", "revia-test@example.invalid"],
        );
        fs::write(path.join("example.rs"), "fn value() -> u8 { 1 }\n").unwrap();
        git(&path, ["add", "example.rs"]);
        git(&path, ["commit", "-qm", "initial"]);
        path
    }

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
    fn committed_anchor_resolves_the_exact_committed_file() {
        let repository = repository();
        let store = AnchorStore::new(&repository);
        let anchor = store
            .committed("HEAD", "example.rs", "@@ -1 +1 @@")
            .unwrap();

        assert_eq!(
            store.resolve_file(&anchor).unwrap(),
            "fn value() -> u8 { 1 }\n"
        );
    }

    #[test]
    fn snapshot_anchor_survives_later_working_tree_edits() {
        let repository = repository();
        fs::write(repository.join("example.rs"), "fn value() -> u8 { 2 }\n").unwrap();
        let store = AnchorStore::new(&repository);
        let anchor = store
            .snapshot_working_tree("example.rs", "@@ -1 +1 @@")
            .unwrap();
        fs::write(repository.join("example.rs"), "fn value() -> u8 { 3 }\n").unwrap();

        assert_eq!(
            store.resolve_file(&anchor).unwrap(),
            "fn value() -> u8 { 2 }\n"
        );
        assert!(
            git(&repository, ["for-each-ref", "refs/revia/snapshots"]).contains(anchor.revision())
        );
    }
}
