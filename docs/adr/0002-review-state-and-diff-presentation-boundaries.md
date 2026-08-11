# ADR 0002: Model review navigation and threads as domain concepts

## Status

Accepted.

## Context

The initial TUI implementation concentrated review-session coordination,
review selection, thread lifecycle mutation, filesystem persistence, split-row
pairing, syntax highlighting, and ratatui drawing in procedural code. That
made a change to a review action or a diff row difficult to inspect without
also reasoning about terminal I/O and JSON writes.

## Concept model

| Concept | Invariants / transitions | Owner | Representation |
| --- | --- | --- | --- |
| Hunk location | A review target is the pair of file path and hunk header; selection never needs to pass them as unrelated strings | `anchor::HunkLocation` | Domain value derived from a parsed diff hunk or an immutable anchor |
| Review cursor | File selection resets hunk/thread; hunk selection resets thread; cursor is allowed to be `(0, 0, 0)` only for an empty diff | `review::ReviewCursor` | Private indices, exposed through intent-named transitions |
| Review session | A cursor always addresses the currently loaded diff, after reload selection is clamped to that diff | `review::ReviewSession` | `LoadedDiff` plus `ReviewCursor` in memory |
| Current-hunk projection | An immutable anchor resolves by exact path/header, then by changed-span overlap and deterministic nearest-start/Git-order tie-break when diff context changes the header | Review mode over the loaded session | Derived only; the anchor and thread JSON remain unchanged |
| Review thread collection | IDs are monotonic; messages append; a needs-attention thread may only be closed by a human; reopen clears close provenance | `thread::ThreadCollection` | Serializable `next_id` + `threads` JSON shape, unchanged from v1 |
| Thread identifier | Thread identity is opaque in domain code while serializing as the same JSON number | `thread::ThreadId` | `#[serde(transparent)]` numeric value |
| Thread persistence | Each successful lifecycle transition replaces one JSON file atomically in the Git common directory | `thread::ThreadStore` | Git-path and filesystem adapter |
| Diff presentation | Removed/added runs pair only for display; one-sided lines remain visible; syntax styling never changes patch content | `presentation` | `DiffLine` to ratatui `Line` conversion |
| Review workflow | Resolves immutable anchors, asks the thread adapter to persist transitions, and interprets review commands | `App` | Application coordination, not concept state |
| Terminal boundary | Raw-mode setup, event polling, key-to-command mapping, and ratatui widget layout | `main.rs` terminal adapter (pending extraction) | Crossterm and ratatui APIs |

## Decision

- Make `HunkLocation`, `ReviewCursor`, `ReviewSession`, and `ThreadId` named
  domain data types. The cursor owns selection transitions; the session owns
  the relationship between a cursor and one loaded diff. Neither depends on
  crossterm, ratatui, Git commands, or filesystem APIs.
- Keep `ThreadCollection` free of Git and filesystem access. It owns all thread
  lifecycle rules and exposes immutable thread state. Current-diff location
  projection belongs beside the loaded review session because it requires
  parsed hunk coordinates; `ThreadStore` persists the same serialized shape
  with the existing atomic temporary-file replacement.
- Put split-row pairing and syntax/highlight conversion in `presentation`.
  The terminal renderer retains widget geometry and consumes presentation rows;
  neither transform can mutate Git patch data or thread state.
- Reduce `App` to coordination: it combines immutable anchors, diff reloads,
  thread persistence, and command interpretation. It must use the shared
  current-hunk projection and collection transitions rather than mutate index
  fields or infer location from rendered rows.

## Contract evidence

| Concern | Evidence | Result / limitation |
| --- | --- | --- |
| CLI flags, raw diff output, diff targets | `cli` and `diff` unit tests plus inspection of `main` composition | Preserved; parsing moved without changing flags or defaults |
| Thread JSON shape and lifecycle | Existing persistence tests plus pure collection lifecycle test | Preserved; fields remain `next_id` and `threads` |
| Context-sensitive anchor projection | U3→U4 header-change and ambiguous split scenario tests | Exact identity remains preferred; derived overlap mapping is deterministic and non-overlapping anchors remain unavailable |
| Review selection transitions | New pure cursor/session tests | Preserved; file/hunk changes reset dependent selection and reload clamps it |
| Human-only close invariant | Collection test and existing store test | Preserved |
| Split/stack presentation | Existing renderer fixture plus `presentation` pairing test | Preserved |
| Atomic write boundary | `ThreadStore::persist` inspection | Preserved; one temporary file then rename per transition |
| Git read-only boundary | Diff/anchor code untouched; no new Git mutation path | Preserved, except existing immutable snapshot refs for anchors |

## Consequences and follow-up

- The codebase has explicit concept ownership for review location, selection,
  session state, thread lifecycle, and presentation transforms, with
  regression coverage independent of the TUI.
- `App` remains the v1 use-case coordinator. The terminal adapter still lives
  in `main.rs`; a future extraction of rendering must expose an intentional
  read-only session view rather than making fields public across the crate. It
  must preserve the documented Hunk-compatible key contract.
- No SQL, generated artifacts, HTTP routes, or database transactions exist in
  this repository, so those checklist items are not applicable.
