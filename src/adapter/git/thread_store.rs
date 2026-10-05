use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};

use crate::domain::thread::ThreadState;

pub struct ThreadRepository {
    path: PathBuf,
}

impl ThreadRepository {
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "retained for post-0.1 thread persistence")
    )]
    pub fn open(repository: &Path) -> Result<(Self, ThreadState)> {
        let common_dir = git_common_dir(repository)?;
        let path = common_dir.join("revia").join("threads.json");
        let state = match fs::read(&path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).context("could not parse revia thread store")?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => ThreadState::default(),
            Err(error) => return Err(error).context("could not read revia thread store"),
        };
        Ok((Self { path }, state))
    }

    pub fn persist(&self, state: &ThreadState) -> Result<()> {
        let parent = self.path.parent().expect("thread store has a parent");
        fs::create_dir_all(parent).context("could not create revia store directory")?;
        let temporary = self.path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(state)?)
            .context("could not write thread store")?;
        fs::rename(temporary, &self.path).context("could not atomically replace thread store")
    }
}

pub fn now_ms() -> Result<u128> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_millis())
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "retained for post-0.1 thread persistence")
)]
fn git_common_dir(repository: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        .context("could not locate Git directory")?;
    if !output.status.success() {
        bail!(
            "not a Git repository: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(PathBuf::from(
        String::from_utf8(output.stdout)
            .context("Git path is not UTF-8")?
            .trim(),
    ))
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use crate::domain::{
        anchor::{Anchor, HunkLocation},
        thread::{Participant, ParticipantKind, Resolution},
    };
    use crate::test_support::temp_dir;

    use super::{ThreadRepository, now_ms};

    fn repository() -> tempfile::TempDir {
        let temporary = temp_dir("thread");
        assert!(
            Command::new("git")
                .arg("init")
                .arg("-q")
                .arg(temporary.path())
                .status()
                .unwrap()
                .success()
        );
        temporary
    }

    fn human() -> Participant {
        Participant {
            id: "human".into(),
            kind: ParticipantKind::Human,
        }
    }

    fn agent() -> Participant {
        Participant {
            id: "agent".into(),
            kind: ParticipantKind::Agent,
        }
    }

    fn anchor() -> Anchor {
        Anchor::new("deadbeef", HunkLocation::new("src/lib.rs", "@@ -1 +1 @@"))
    }

    #[test]
    fn persists_orthogonal_lifecycle_and_enforces_human_escalation() {
        let repository = repository();
        let (repository_adapter, mut state) = ThreadRepository::open(repository.path()).unwrap();
        let id = state.post(
            anchor(),
            human(),
            "Please check this.".into(),
            now_ms().unwrap(),
        );
        state.set_outdated(id, true).unwrap();
        state.set_needs_attention(id, true).unwrap();
        assert!(state.close(id, &agent()).is_err());
        state.close(id, &human()).unwrap();
        repository_adapter.persist(&state).unwrap();

        let (_, restored) = ThreadRepository::open(repository.path()).unwrap();
        let thread = &restored.threads()[0];
        assert_eq!(thread.resolution, Resolution::Resolved);
        assert!(thread.outdated);
        assert!(thread.needs_attention);
    }

    #[test]
    fn normal_threads_can_be_closed_and_reopened_by_any_participant() {
        let repository = repository();
        let (repository_adapter, mut state) = ThreadRepository::open(repository.path()).unwrap();
        let id = state.post(
            anchor(),
            human(),
            "Initial review".into(),
            now_ms().unwrap(),
        );
        state
            .reply(id, agent(), "I disagree.".into(), now_ms().unwrap())
            .unwrap();
        state.close(id, &agent()).unwrap();
        state.reopen(id).unwrap();
        repository_adapter.persist(&state).unwrap();

        let thread = &state.threads()[0];
        assert_eq!(thread.resolution, Resolution::Open);
        assert_eq!(thread.messages.len(), 2);
    }
}
