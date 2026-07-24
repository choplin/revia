use crate::{
    anchor::HunkLocation,
    diff::{DiffRequest, LoadedDiff},
    mode::{ActiveMode, review::ReloadPurpose},
    thread::{ThreadChange, ThreadId, ThreadOperation},
};

/// Root effect vocabulary, organized by external operation rather than Mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    ReloadDiff {
        owner: ActiveMode,
        request: DiffRequest,
        purpose: ReloadPurpose,
    },
    ChangeThreads {
        owner: ActiveMode,
        operation: ThreadOperation,
    },
    ResolveThread {
        owner: ActiveMode,
        id: ThreadId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectResult {
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
