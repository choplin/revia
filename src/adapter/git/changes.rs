use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};

static TEMPORARY_DIFF_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Builds one Git-compatible patch for every untracked path without hashing
/// content into the repository or spawning one process per file.
pub fn untracked_patch(repository: &Path, context_lines: usize) -> Result<String> {
    let repository_root = repository_root(repository)?;
    let paths = untracked_paths(&repository_root)?;
    if paths.is_empty() {
        return Ok(String::new());
    }

    let temporary = TemporaryDiff::new(&repository_root)?;
    for path in paths {
        temporary.link(&repository_root, &path)?;
    }

    let output = Command::new("git")
        .args(["-c", "core.quotePath=true"])
        .arg("-C")
        .arg(temporary.root())
        .env("GIT_CEILING_DIRECTORIES", &repository_root)
        .args([
            "diff",
            "--no-index",
            "--no-ext-diff",
            &format!("--unified={context_lines}"),
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "--",
            "empty",
            "new",
        ])
        .output()
        .context("could not create the untracked-file patch")?;
    if !matches!(output.status.code(), Some(0 | 1)) {
        bail!(
            "git could not create the untracked-file patch: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let text = String::from_utf8(output.stdout).context("git produced a non-UTF-8 diff")?;
    Ok(normalize_mirror_paths(&text))
}

pub fn repository_root(repository: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .with_context(|| format!("could not inspect repository at {}", repository.display()))?;
    if !output.status.success() {
        bail!(
            "git could not resolve the repository root: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let mut path = output.stdout;
    while matches!(path.last(), Some(b'\n' | b'\r')) {
        path.pop();
    }
    path_from_bytes(&path)
}

fn untracked_paths(repository: &Path) -> Result<Vec<PathBuf>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["ls-files", "--others", "--exclude-standard", "-z", "--"])
        .output()
        .context("could not list untracked files")?;
    if !output.status.success() {
        bail!(
            "git could not list untracked files: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(path_from_bytes)
        .collect()
}

#[cfg(unix)]
fn path_from_bytes(path: &[u8]) -> Result<PathBuf> {
    use std::os::unix::ffi::OsStringExt;

    Ok(std::ffi::OsString::from_vec(path.to_vec()).into())
}

#[cfg(not(unix))]
fn path_from_bytes(path: &[u8]) -> Result<PathBuf> {
    String::from_utf8(path.to_vec())
        .map(PathBuf::from)
        .context("an untracked path is not valid UTF-8")
}

fn normalize_mirror_paths(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        if line.starts_with("diff --git ")
            || line.starts_with("+++ ")
            || line.starts_with("Binary files ")
        {
            normalized.push_str(
                &line
                    .replace("a/empty/", "a/")
                    .replace("a/new/", "a/")
                    .replace("b/new/", "b/"),
            );
        } else {
            normalized.push_str(line);
        }
    }
    normalized
}

struct TemporaryDiff {
    root: PathBuf,
    mirror: PathBuf,
}

impl TemporaryDiff {
    fn new(repository: &Path) -> Result<Self> {
        let mut parents = vec![repository.to_owned()];
        if let Some(parent) = repository.parent() {
            parents.push(parent.to_owned());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;

            let device = fs::metadata(repository)?.dev();
            parents.retain(|candidate| {
                fs::metadata(candidate).is_ok_and(|metadata| metadata.dev() == device)
            });
        }
        let mut last_error = None;
        for parent in parents {
            for _ in 0..100 {
                let nonce = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .context("system clock is before Unix epoch")?
                    .as_nanos();
                let sequence = TEMPORARY_DIFF_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let root = parent.join(format!(
                    ".revia-untracked-{}-{nonce}-{sequence}",
                    std::process::id()
                ));
                let mut builder = fs::DirBuilder::new();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    builder.mode(0o700);
                }
                match builder.create(&root) {
                    Ok(()) => {
                        fs::create_dir(root.join("empty"))?;
                        let mirror = root.join("new");
                        fs::create_dir(&mirror)?;
                        return Ok(Self { root, mirror });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => {
                        last_error = Some(error);
                        break;
                    }
                }
            }
        }
        Err(last_error.unwrap_or_else(|| std::io::Error::other("unique name attempts exhausted")))
            .context("could not create a temporary diff mirror")
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn link(&self, repository: &Path, relative: &Path) -> Result<()> {
        let source = repository.join(relative);
        let destination = self.mirror.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let metadata = fs::symlink_metadata(&source)?;
        if metadata.file_type().is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(fs::read_link(&source)?, &destination)?;
            #[cfg(not(unix))]
            bail!("untracked symbolic links are unsupported on this platform");
        } else {
            fs::hard_link(&source, &destination).with_context(|| {
                format!("could not mirror untracked path {}", relative.display())
            })?;
        }
        Ok(())
    }
}

impl Drop for TemporaryDiff {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
