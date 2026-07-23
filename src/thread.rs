use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::anchor::{Anchor, HunkLocation};

/// Opaque identity for one persisted review thread.
///
/// It remains a JSON number so existing thread stores do not need migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ThreadId(u64);

impl std::fmt::Display for ThreadId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

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
    pub id: ThreadId,
    pub anchor: Anchor,
    pub messages: Vec<Message>,
    pub resolution: Resolution,
    pub closed_by: Option<Participant>,
    /// A display hint only; it never changes resolution.
    pub outdated: bool,
    /// Human escalation; agents may not close a thread while it is set.
    pub needs_attention: bool,
}

/// Pure, persisted review-thread state.
///
/// This owns lifecycle transitions and deliberately has no filesystem or Git
/// dependency. `ThreadStore` below is the adapter that makes each transition
/// durable in the repository's common Git directory.
#[derive(Debug, Default, Serialize, Deserialize)]
struct ThreadCollection {
    next_id: u64,
    threads: Vec<ReviewThread>,
}

impl ThreadCollection {
    fn threads(&self) -> &[ReviewThread] {
        &self.threads
    }

    fn post(
        &mut self,
        anchor: Anchor,
        author: Participant,
        body: String,
        created_at_ms: u128,
    ) -> ThreadId {
        let id = ThreadId(self.next_id);
        self.next_id += 1;
        self.threads.push(ReviewThread {
            id,
            anchor,
            messages: vec![Message {
                author,
                body,
                created_at_ms,
            }],
            resolution: Resolution::Open,
            closed_by: None,
            outdated: false,
            needs_attention: false,
        });
        id
    }

    fn reply(
        &mut self,
        id: ThreadId,
        author: Participant,
        body: String,
        created_at_ms: u128,
    ) -> Result<()> {
        self.thread_mut(id)?.messages.push(Message {
            author,
            body,
            created_at_ms,
        });
        Ok(())
    }

    fn close(&mut self, id: ThreadId, actor: &Participant) -> Result<()> {
        let thread = self.thread_mut(id)?;
        if thread.needs_attention && actor.kind != ParticipantKind::Human {
            bail!("only a human can close a needs-attention thread");
        }
        thread.resolution = Resolution::Resolved;
        thread.closed_by = Some(actor.clone());
        Ok(())
    }

    fn reopen(&mut self, id: ThreadId) -> Result<()> {
        let thread = self.thread_mut(id)?;
        thread.resolution = Resolution::Open;
        thread.closed_by = None;
        Ok(())
    }

    fn set_outdated(&mut self, id: ThreadId, outdated: bool) -> Result<()> {
        self.thread_mut(id)?.outdated = outdated;
        Ok(())
    }

    fn set_needs_attention(&mut self, id: ThreadId, value: bool) -> Result<()> {
        self.thread_mut(id)?.needs_attention = value;
        Ok(())
    }

    fn thread_mut(&mut self, id: ThreadId) -> Result<&mut ReviewThread> {
        self.threads
            .iter_mut()
            .find(|thread| thread.id == id)
            .context("thread does not exist")
    }

    fn thread(&self, id: ThreadId) -> Option<&ReviewThread> {
        self.threads.iter().find(|thread| thread.id == id)
    }

    fn at(&self, location: &HunkLocation) -> Vec<&ReviewThread> {
        self.threads
            .iter()
            .filter(|thread| thread.anchor.is_at(location))
            .collect()
    }

    fn in_file(&self, path: &str) -> Vec<&ReviewThread> {
        self.threads
            .iter()
            .filter(|thread| thread.anchor.location().path() == path)
            .collect()
    }

    fn ordered_ids(&self) -> Vec<ThreadId> {
        let mut threads = self.threads.iter().collect::<Vec<_>>();
        threads.sort_by_key(|thread| {
            if thread.needs_attention {
                0
            } else if matches!(thread.resolution, Resolution::Open) {
                1
            } else {
                2
            }
        });
        threads.into_iter().map(|thread| thread.id).collect()
    }

    fn attention_ids(&self) -> Vec<ThreadId> {
        self.ordered_ids()
            .into_iter()
            .filter(|id| {
                self.thread(*id)
                    .is_some_and(|thread| thread.needs_attention)
            })
            .collect()
    }
}

pub struct ThreadStore {
    path: PathBuf,
    collection: ThreadCollection,
}

impl ThreadStore {
    pub fn open(repository: &Path) -> Result<Self> {
        let common_dir = git_common_dir(repository)?;
        let path = common_dir.join("revia").join("threads.json");
        let collection = match fs::read(&path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).context("could not parse revia thread store")?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ThreadCollection::default()
            }
            Err(error) => return Err(error).context("could not read revia thread store"),
        };
        Ok(Self { path, collection })
    }

    pub fn threads(&self) -> &[ReviewThread] {
        self.collection.threads()
    }

    pub fn post(
        &mut self,
        anchor: Anchor,
        author: Participant,
        body: impl Into<String>,
    ) -> Result<ThreadId> {
        let id = self.collection.post(anchor, author, body.into(), now_ms()?);
        self.persist()?;
        Ok(id)
    }

    pub fn reply(
        &mut self,
        id: ThreadId,
        author: Participant,
        body: impl Into<String>,
    ) -> Result<()> {
        self.collection.reply(id, author, body.into(), now_ms()?)?;
        self.persist()
    }

    pub fn close(&mut self, id: ThreadId, actor: &Participant) -> Result<()> {
        self.collection.close(id, actor)?;
        self.persist()
    }

    pub fn reopen(&mut self, id: ThreadId) -> Result<()> {
        self.collection.reopen(id)?;
        self.persist()
    }
    pub fn set_outdated(&mut self, id: ThreadId, outdated: bool) -> Result<()> {
        self.collection.set_outdated(id, outdated)?;
        self.persist()
    }
    pub fn set_needs_attention(&mut self, id: ThreadId, value: bool) -> Result<()> {
        self.collection.set_needs_attention(id, value)?;
        self.persist()
    }

    pub fn thread(&self, id: ThreadId) -> Option<&ReviewThread> {
        self.collection.thread(id)
    }

    pub fn threads_at(&self, location: &HunkLocation) -> Vec<&ReviewThread> {
        self.collection.at(location)
    }

    pub fn threads_in_file(&self, path: &str) -> Vec<&ReviewThread> {
        self.collection.in_file(path)
    }

    pub fn ordered_ids(&self) -> Vec<ThreadId> {
        self.collection.ordered_ids()
    }

    pub fn attention_ids(&self) -> Vec<ThreadId> {
        self.collection.attention_ids()
    }

    fn persist(&self) -> Result<()> {
        let parent = self.path.parent().expect("thread store has a parent");
        fs::create_dir_all(parent).context("could not create revia store directory")?;
        let temporary = self.path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(&self.collection)?)
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
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::anchor::{Anchor, HunkLocation};

    use super::{Participant, ParticipantKind, Resolution, ThreadCollection, ThreadStore};

    static REPOSITORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn repository() -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = REPOSITORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("revia-thread-{nonce}-{sequence}"));
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
        Anchor::new("deadbeef", HunkLocation::new("src/lib.rs", "@@ -1 +1 @@"))
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

    #[test]
    fn collection_owns_lifecycle_without_persistence_dependencies() {
        let mut collection = ThreadCollection::default();
        let id = collection.post(anchor(), human(), "Initial review".into(), 1);
        collection.reply(id, agent(), "Reply".into(), 2).unwrap();
        collection.set_needs_attention(id, true).unwrap();
        assert!(collection.close(id, &agent()).is_err());
        collection.close(id, &human()).unwrap();

        let thread = &collection.threads()[0];
        assert_eq!(thread.resolution, Resolution::Resolved);
        assert_eq!(thread.messages.len(), 2);
        let persisted = serde_json::to_value(&collection).unwrap();
        assert_eq!(persisted["next_id"], 1);
        assert_eq!(persisted["threads"][0]["id"], 0);
    }
}
