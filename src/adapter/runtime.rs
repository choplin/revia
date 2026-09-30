use std::path::{Path, PathBuf};

use crate::{
    adapter::git::{
        anchor::AnchorStore,
        diff as git_diff,
        thread_store::{ThreadRepository, now_ms},
    },
    app::{Effect, EffectResult, Outcome},
    domain::{
        diff::DiffTarget,
        thread::{
            Participant, ParticipantKind, ThreadChange, ThreadOperation, ThreadState, ThreadSuccess,
        },
    },
};

pub struct Runtime {
    repository: PathBuf,
    threads: ThreadRepository,
}

impl Runtime {
    pub fn open(repository: &Path) -> anyhow::Result<(Self, ThreadState)> {
        let (threads, state) = ThreadRepository::open(repository)?;
        Ok((
            Self {
                repository: repository.to_owned(),
                threads,
            },
            state,
        ))
    }

    pub fn perform(&self, effect: Effect, current_threads: &ThreadState) -> EffectResult {
        match effect {
            Effect::ReloadDiff {
                operation_id,
                owner,
                request,
                purpose,
            } => EffectResult {
                operation_id,
                owner,
                outcome: Outcome::DiffReloaded {
                    purpose,
                    result: git_diff::load(&self.repository, &request)
                        .map_err(|error| error.to_string()),
                },
            },
            Effect::ChangeThreads {
                operation_id,
                owner,
                operation,
            } => EffectResult {
                operation_id,
                owner,
                outcome: Outcome::ThreadsChanged {
                    result: self.change_threads(current_threads, operation),
                },
            },
            Effect::ResolveThread {
                operation_id,
                owner,
                id,
            } => EffectResult {
                operation_id,
                owner,
                outcome: Outcome::ThreadResolved {
                    id,
                    result: self.resolve_thread(current_threads, id),
                },
            },
        }
    }

    fn resolve_thread(
        &self,
        current_threads: &ThreadState,
        id: crate::domain::thread::ThreadId,
    ) -> Result<crate::domain::anchor::HunkLocation, String> {
        current_threads
            .thread(id)
            .ok_or_else(|| "thread does not exist".to_owned())
            .and_then(|thread| {
                let anchor = thread.anchor.clone();
                AnchorStore::new(&self.repository)
                    .resolve_file(&anchor)
                    .map(|_| anchor.location())
                    .map_err(|error| error.to_string())
            })
    }

    fn change_threads(
        &self,
        current: &ThreadState,
        operation: ThreadOperation,
    ) -> Result<ThreadChange, String> {
        let mut state = current.clone();
        let success = match operation {
            ThreadOperation::Submit {
                target,
                location,
                body,
                reply_to,
            } => {
                if let Some(id) = reply_to {
                    state
                        .reply(
                            id,
                            human(),
                            body,
                            now_ms().map_err(|error| error.to_string())?,
                        )
                        .map_err(|error| format!("could not post thread: {error}"))?;
                    ThreadSuccess::Replied
                } else {
                    let anchors = AnchorStore::new(&self.repository);
                    let anchor = match target {
                        DiffTarget::Commit(revision) => {
                            anchors.committed(&revision, location.path(), location.hunk_header())
                        }
                        DiffTarget::WorkingTree | DiffTarget::Staged | DiffTarget::Range(_) => {
                            anchors.snapshot_working_tree(location.path(), location.hunk_header())
                        }
                    }
                    .map_err(|error| format!("could not post thread: {error}"))?;
                    let id = state.post(
                        anchor,
                        human(),
                        body,
                        now_ms().map_err(|error| error.to_string())?,
                    );
                    ThreadSuccess::Posted(id)
                }
            }
            ThreadOperation::Close { id } => {
                state
                    .close(id, &human())
                    .map_err(|error| format!("could not close thread: {error}"))?;
                ThreadSuccess::Closed
            }
            ThreadOperation::Reopen { id } => {
                state
                    .reopen(id)
                    .map_err(|error| format!("could not reopen thread: {error}"))?;
                ThreadSuccess::Reopened
            }
            ThreadOperation::SetAttention { id, value } => {
                state
                    .set_needs_attention(id, value)
                    .map_err(|error| format!("could not update thread: {error}"))?;
                ThreadSuccess::AttentionToggled
            }
            ThreadOperation::SetOutdated { id, value } => {
                state
                    .set_outdated(id, value)
                    .map_err(|error| format!("could not update thread: {error}"))?;
                ThreadSuccess::OutdatedToggled
            }
        };
        self.threads
            .persist(&state)
            .map_err(|error| match success {
                ThreadSuccess::Posted(_) | ThreadSuccess::Replied => {
                    format!("could not post thread: {error}")
                }
                ThreadSuccess::Closed => format!("could not close thread: {error}"),
                ThreadSuccess::Reopened => format!("could not reopen thread: {error}"),
                ThreadSuccess::AttentionToggled | ThreadSuccess::OutdatedToggled => {
                    format!("could not update thread: {error}")
                }
            })?;
        Ok(ThreadChange { state, success })
    }
}

fn human() -> Participant {
    Participant {
        id: "human".into(),
        kind: ParticipantKind::Human,
    }
}
