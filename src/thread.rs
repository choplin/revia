use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::anchor::Anchor;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParticipantKind {
    Human,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Participant {
    pub id: String,
    pub kind: ParticipantKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resolution {
    Open,
    Resolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub author: Participant,
    pub body: String,
    pub created_at_ms: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewThread {
    pub id: u64,
    pub anchor: Anchor,
    pub messages: Vec<Message>,
    pub resolution: Resolution,
    pub closed_by: Option<Participant>,
    /// A display hint only; it never changes resolution.
    pub outdated: bool,
    /// Human escalation; agents may not close a thread while it is set.
    pub needs_attention: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ThreadFile {
    next_id: u64,
    threads: Vec<ReviewThread>,
}

pub struct ThreadStore {
    path: PathBuf,
    file: ThreadFile,
}

impl ThreadStore {
    pub fn open(repository: &Path) -> Result<Self> {
        let common_dir = git_common_dir(repository)?;
        let path = common_dir.join("revia").join("threads.json");
        let file = match fs::read(&path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).context("could not parse revia thread store")?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => ThreadFile::default(),
            Err(error) => return Err(error).context("could not read revia thread store"),
        };
        Ok(Self { path, file })
    }

    pub fn threads(&self) -> &[ReviewThread] {
        &self.file.threads
    }

    pub fn post(
        &mut self,
        anchor: Anchor,
        author: Participant,
        body: impl Into<String>,
    ) -> Result<u64> {
        let id = self.file.next_id;
        self.file.next_id += 1;
        self.file.threads.push(ReviewThread {
            id,
            anchor,
            messages: vec![Message {
                author,
                body: body.into(),
                created_at_ms: now_ms()?,
            }],
            resolution: Resolution::Open,
            closed_by: None,
            outdated: false,
            needs_attention: false,
        });
        self.persist()?;
        Ok(id)
    }

    pub fn reply(&mut self, id: u64, author: Participant, body: impl Into<String>) -> Result<()> {
        self.thread_mut(id)?.messages.push(Message {
            author,
            body: body.into(),
            created_at_ms: now_ms()?,
        });
        self.persist()
    }

    pub fn close(&mut self, id: u64, actor: &Participant) -> Result<()> {
        let thread = self.thread_mut(id)?;
        if thread.needs_attention && actor.kind != ParticipantKind::Human {
            bail!("only a human can close a needs-attention thread");
        }
        thread.resolution = Resolution::Resolved;
        thread.closed_by = Some(actor.clone());
        self.persist()
    }

    pub fn reopen(&mut self, id: u64) -> Result<()> {
        let thread = self.thread_mut(id)?;
        thread.resolution = Resolution::Open;
        thread.closed_by = None;
        self.persist()
    }
    pub fn set_outdated(&mut self, id: u64, outdated: bool) -> Result<()> {
        self.thread_mut(id)?.outdated = outdated;
        self.persist()
    }
    pub fn set_needs_attention(&mut self, id: u64, value: bool) -> Result<()> {
        self.thread_mut(id)?.needs_attention = value;
        self.persist()
    }

    fn thread_mut(&mut self, id: u64) -> Result<&mut ReviewThread> {
        self.file
            .threads
            .iter_mut()
            .find(|thread| thread.id == id)
            .context("thread does not exist")
    }

    fn persist(&self) -> Result<()> {
        let parent = self.path.parent().expect("thread store has a parent");
        fs::create_dir_all(parent).context("could not create revia store directory")?;
        let temporary = self.path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(&self.file)?)
            .context("could not write thread store")?;
        fs::rename(temporary, &self.path).context("could not atomically replace thread store")
    }
}

fn now_ms() -> Result<u128> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_millis())
}

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
    use std::{
        fs,
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::anchor::Anchor;

    use super::{Participant, ParticipantKind, Resolution, ThreadStore};

    fn repository() -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("revia-thread-{nonce}"));
        fs::create_dir_all(&path).unwrap();
        assert!(
            Command::new("git")
                .arg("init")
                .arg("-q")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        path
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
        Anchor {
            revision: "deadbeef".into(),
            path: "src/lib.rs".into(),
            hunk_header: "@@ -1 +1 @@".into(),
        }
    }

    #[test]
    fn persists_orthogonal_lifecycle_and_enforces_human_escalation() {
        let repository = repository();
        let mut store = ThreadStore::open(&repository).unwrap();
        let id = store.post(anchor(), human(), "Please check this.").unwrap();
        store.set_outdated(id, true).unwrap();
        store.set_needs_attention(id, true).unwrap();
        assert!(store.close(id, &agent()).is_err());
        store.close(id, &human()).unwrap();

        let restored = ThreadStore::open(&repository).unwrap();
        let thread = &restored.threads()[0];
        assert_eq!(thread.resolution, Resolution::Resolved);
        assert!(thread.outdated);
        assert!(thread.needs_attention);
    }

    #[test]
    fn normal_threads_can_be_closed_and_reopened_by_any_participant() {
        let repository = repository();
        let mut store = ThreadStore::open(&repository).unwrap();
        let id = store.post(anchor(), human(), "Initial review").unwrap();
        store.reply(id, agent(), "I disagree.").unwrap();
        store.close(id, &agent()).unwrap();
        store.reopen(id).unwrap();

        let thread = &store.threads()[0];
        assert_eq!(thread.resolution, Resolution::Open);
        assert_eq!(thread.messages.len(), 2);
    }
}
