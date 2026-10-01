//! Review-thread state, lifecycle rules, and application operations.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::{
    anchor::{Anchor, HunkLocation},
    diff::{DiffProvenance, DiffSource},
};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadOperation {
    Submit {
        source: DiffSource,
        provenance: DiffProvenance,
        location: HunkLocation,
        body: String,
        reply_to: Option<ThreadId>,
    },
    Close {
        id: ThreadId,
    },
    Reopen {
        id: ThreadId,
    },
    SetAttention {
        id: ThreadId,
        value: bool,
    },
    SetOutdated {
        id: ThreadId,
        value: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadChange {
    pub state: ThreadState,
    pub success: ThreadSuccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadSuccess {
    Posted(ThreadId),
    Replied,
    Closed,
    Reopened,
    AttentionToggled,
    OutdatedToggled,
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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadState {
    next_id: u64,
    threads: Vec<ReviewThread>,
}

impl ThreadState {
    pub fn threads(&self) -> &[ReviewThread] {
        &self.threads
    }

    pub fn post(
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

    pub fn reply(
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

    pub fn close(&mut self, id: ThreadId, actor: &Participant) -> Result<()> {
        let thread = self.thread_mut(id)?;
        if thread.needs_attention && actor.kind != ParticipantKind::Human {
            bail!("only a human can close a needs-attention thread");
        }
        thread.resolution = Resolution::Resolved;
        thread.closed_by = Some(actor.clone());
        Ok(())
    }

    pub fn reopen(&mut self, id: ThreadId) -> Result<()> {
        let thread = self.thread_mut(id)?;
        thread.resolution = Resolution::Open;
        thread.closed_by = None;
        Ok(())
    }

    pub fn set_outdated(&mut self, id: ThreadId, outdated: bool) -> Result<()> {
        self.thread_mut(id)?.outdated = outdated;
        Ok(())
    }

    pub fn set_needs_attention(&mut self, id: ThreadId, value: bool) -> Result<()> {
        self.thread_mut(id)?.needs_attention = value;
        Ok(())
    }

    fn thread_mut(&mut self, id: ThreadId) -> Result<&mut ReviewThread> {
        self.threads
            .iter_mut()
            .find(|thread| thread.id == id)
            .context("thread does not exist")
    }

    pub fn thread(&self, id: ThreadId) -> Option<&ReviewThread> {
        self.threads.iter().find(|thread| thread.id == id)
    }

    pub fn ordered_ids(&self) -> Vec<ThreadId> {
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
}

#[cfg(test)]
mod tests {
    use super::{Participant, ParticipantKind, Resolution, ThreadState};
    use crate::domain::anchor::{Anchor, HunkLocation};

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
    fn collection_owns_lifecycle_without_persistence_dependencies() {
        let mut collection = ThreadState::default();
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
