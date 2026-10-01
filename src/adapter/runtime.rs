use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use crate::{
    adapter::{
        diff as source_diff,
        git::{
            anchor::AnchorStore,
            thread_store::{ThreadRepository, now_ms},
        },
    },
    app::{Effect, EffectResult, Outcome},
    domain::{
        diff::{DiffProvenance, DiffRequest, DiffSource, GitComparison, LoadedDiff, PatchInput},
        thread::{
            Participant, ParticipantKind, ThreadChange, ThreadOperation, ThreadState, ThreadSuccess,
        },
    },
};

pub struct Runtime {
    repository: PathBuf,
    threads: Option<ThreadRepository>,
    stdin_patch: Option<LoadedDiff>,
    snapshot_cache: Mutex<Option<SnapshotCache>>,
}

struct SnapshotCache {
    source: DiffSource,
    expected_document: Arc<crate::domain::diff::DiffDocument>,
    object: String,
}

impl Runtime {
    pub fn open(
        repository: &Path,
        request: &DiffRequest,
        initial_diff: &LoadedDiff,
    ) -> anyhow::Result<(Self, ThreadState)> {
        let (threads, state) = if request.source.supports_persistent_threads() {
            let (repository, state) = ThreadRepository::open(repository)?;
            (Some(repository), state)
        } else {
            (None, ThreadState::default())
        };
        let stdin_patch = matches!(request.source, DiffSource::Patch(PatchInput::Stdin))
            .then(|| initial_diff.clone());
        Ok((
            Self {
                repository: repository.to_owned(),
                threads,
                stdin_patch,
                snapshot_cache: Mutex::new(None),
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
                    result: self.reload(&request).map_err(|error| error.to_string()),
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

    fn reload(&self, request: &DiffRequest) -> anyhow::Result<LoadedDiff> {
        if matches!(request.source, DiffSource::Patch(PatchInput::Stdin)) {
            return self.stdin_patch.clone().ok_or_else(|| {
                anyhow::anyhow!("the original stdin patch is unavailable for reload")
            });
        }
        let loaded = source_diff::load(&self.repository, request)?;
        *self
            .snapshot_cache
            .lock()
            .map_err(|_| anyhow::anyhow!("the snapshot cache is unavailable"))? = None;
        Ok(loaded)
    }

    fn resolve_thread(
        &self,
        current_threads: &ThreadState,
        id: crate::domain::thread::ThreadId,
    ) -> Result<crate::domain::anchor::HunkLocation, String> {
        if self.threads.is_none() {
            return Err(patch_persistence_error());
        }
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
        let threads = self.threads.as_ref().ok_or_else(patch_persistence_error)?;
        let mut state = current.clone();
        let success = match operation {
            ThreadOperation::Submit {
                source,
                provenance,
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
                    let anchor = match source {
                        DiffSource::Git(_) => match provenance {
                            DiffProvenance::GitObject(object) => {
                                anchors.object(&object, location.path(), location.hunk_header())
                            }
                            DiffProvenance::MutableGit {
                                expected_document,
                                context_lines,
                            } => self
                                .snapshot_mutable(&source, &expected_document, context_lines)
                                .and_then(|object| {
                                    anchors.object(&object, location.path(), location.hunk_header())
                                }),
                            DiffProvenance::None => Err(anyhow::anyhow!(
                                "the loaded diff has no immutable Git provenance"
                            )),
                        },
                        DiffSource::Patch(_) => return Err(patch_persistence_error()),
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
        threads.persist(&state).map_err(|error| match success {
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

    fn snapshot_mutable(
        &self,
        source: &DiffSource,
        expected_document: &Arc<crate::domain::diff::DiffDocument>,
        context_lines: usize,
    ) -> anyhow::Result<String> {
        let mut cache = self
            .snapshot_cache
            .lock()
            .map_err(|_| anyhow::anyhow!("the snapshot cache is unavailable"))?;
        if let Some(cached) = cache.as_ref()
            && &cached.source == source
            && cached.expected_document.as_ref() == expected_document.as_ref()
        {
            return Ok(cached.object.clone());
        }

        let store = AnchorStore::new(&self.repository);
        let object = match source {
            DiffSource::Git(GitComparison::Changes) => store.snapshot_changes(),
            DiffSource::Git(GitComparison::Staged) => store.snapshot_index(),
            DiffSource::Git(GitComparison::Unstaged) => store.snapshot_working_tree(),
            DiffSource::Git(GitComparison::Revision(_) | GitComparison::Range(_)) => {
                anyhow::bail!("immutable Git comparisons do not require a snapshot")
            }
            DiffSource::Patch(_) => anyhow::bail!("patch input cannot create a Git snapshot"),
        }
        .map_err(|error| {
            anyhow::anyhow!(
                "this comparison cannot be persisted as an immutable Git snapshot: {error}"
            )
        })?;
        let DiffSource::Git(comparison) = source else {
            unreachable!("patch input returned before snapshot verification")
        };
        let snapshot = crate::adapter::git::diff::snapshot_document(
            &self.repository,
            comparison,
            &object,
            context_lines,
        )?;
        if !same_evidence(expected_document, &snapshot) {
            anyhow::bail!(
                "this comparison cannot be persisted because an immutable Git snapshot cannot represent the displayed evidence"
            );
        }
        *cache = Some(SnapshotCache {
            source: source.clone(),
            expected_document: expected_document.clone(),
            object: object.clone(),
        });
        Ok(object)
    }
}

fn same_evidence(
    left: &crate::domain::diff::DiffDocument,
    right: &crate::domain::diff::DiffDocument,
) -> bool {
    let mut left = left.files.iter().collect::<Vec<_>>();
    let mut right = right.files.iter().collect::<Vec<_>>();
    left.sort_by(|a, b| a.path.cmp(&b.path));
    right.sort_by(|a, b| a.path.cmp(&b.path));
    left == right
}

fn patch_persistence_error() -> String {
    DiffSource::Patch(PatchInput::Stdin)
        .persistent_threads_unavailable_reason()
        .expect("patch sources do not support persistent threads")
        .into()
}

fn human() -> Participant {
    Participant {
        id: "human".into(),
        kind: ParticipantKind::Human,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::Path,
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::Runtime;
    use crate::domain::diff::{
        DiffDocument, DiffProvenance, DiffRequest, DiffSource, GitComparison, LoadedDiff,
        PatchInput,
    };

    #[test]
    fn stdin_patch_reload_reuses_the_normalized_initial_source() {
        let text =
            "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-old\n+new\n";
        let initial = LoadedDiff {
            text: text.into(),
            document: DiffDocument::parse(text).into(),
            provenance: DiffProvenance::None,
        };
        let request = DiffRequest {
            source: DiffSource::Patch(PatchInput::Stdin),
            context_lines: 3,
        };
        let (runtime, threads) = Runtime::open(Path::new("."), &request, &initial).unwrap();

        let reloaded = runtime.reload(&request).unwrap();

        assert_eq!(reloaded, initial);
        assert!(threads.threads().is_empty());
    }

    #[test]
    fn mutable_snapshot_matches_the_loaded_diff_and_is_reused_after_later_edits() {
        let repository = std::env::temp_dir().join(format!(
            "revia-runtime-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&repository).unwrap();
        let git = |arguments: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(&repository)
                .args(arguments)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap()
        };
        git(&["init", "-q"]);
        git(&["config", "user.name", "Revia Test"]);
        git(&["config", "user.email", "revia@example.invalid"]);
        fs::write(repository.join("example.txt"), "staged\n").unwrap();
        git(&["add", "example.txt"]);
        fs::write(repository.join("example.txt"), "working\n").unwrap();
        let request = DiffRequest {
            source: DiffSource::Git(GitComparison::Changes),
            context_lines: 3,
        };
        let initial = crate::adapter::diff::load(&repository, &request).unwrap();
        let DiffProvenance::MutableGit {
            expected_document,
            context_lines,
        } = &initial.provenance
        else {
            panic!("changes must carry mutable comparison evidence");
        };
        let (runtime, _) = Runtime::open(&repository, &request, &initial).unwrap();

        let object = runtime
            .snapshot_mutable(&request.source, expected_document, *context_lines)
            .unwrap();
        fs::write(repository.join("example.txt"), "later\n").unwrap();
        let reused = runtime
            .snapshot_mutable(&request.source, expected_document, *context_lines)
            .unwrap();
        let (fresh_runtime, _) = Runtime::open(&repository, &request, &initial).unwrap();
        let stale =
            fresh_runtime.snapshot_mutable(&request.source, expected_document, *context_lines);

        assert_eq!(object, reused);
        assert_eq!(
            git(&["show", &format!("{object}:example.txt")]),
            "working\n"
        );
        assert!(
            stale
                .unwrap_err()
                .to_string()
                .contains("cannot represent the displayed evidence")
        );
        runtime.reload(&request).unwrap();
        assert!(runtime.snapshot_cache.lock().unwrap().is_none());
        fs::remove_dir_all(repository).unwrap();
    }
}
