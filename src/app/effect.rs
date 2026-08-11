use crate::{
    anchor::HunkLocation,
    diff::{DiffRequest, LoadedDiff},
    mode::{ActiveMode, review::ReloadPurpose},
    thread::{ThreadChange, ThreadId, ThreadOperation},
};

pub type OperationId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingEffectKind {
    ReloadDiff,
    ChangeThreads,
    ResolveThread,
}

/// Root effect vocabulary, organized by external operation rather than Mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    ReloadDiff {
        operation_id: OperationId,
        owner: ActiveMode,
        request: DiffRequest,
        purpose: ReloadPurpose,
    },
    ChangeThreads {
        operation_id: OperationId,
        owner: ActiveMode,
        operation: ThreadOperation,
    },
    ResolveThread {
        operation_id: OperationId,
        owner: ActiveMode,
        id: ThreadId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectResult {
    pub operation_id: OperationId,
    pub owner: ActiveMode,
    pub outcome: Outcome,
}

/// Runtime outcomes, organized by external operation rather than Mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    DiffReloaded {
        purpose: ReloadPurpose,
        result: Result<LoadedDiff, String>,
    },
    ThreadsChanged {
        result: Result<ThreadChange, String>,
    },
    ThreadResolved {
        id: ThreadId,
        result: Result<HunkLocation, String>,
    },
}

impl Outcome {
    pub fn pending_kind(&self) -> PendingEffectKind {
        match self {
            Self::DiffReloaded { .. } => PendingEffectKind::ReloadDiff,
            Self::ThreadsChanged { .. } => PendingEffectKind::ChangeThreads,
            Self::ThreadResolved { .. } => PendingEffectKind::ResolveThread,
        }
    }
}
