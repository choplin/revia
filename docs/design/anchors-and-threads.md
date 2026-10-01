# Immutable Anchors and Thread Lifecycle

## Rule

Every persistent thread is stored against an immutable Git object, repository
path, and hunk header. Acquisition resolves immutable comparisons immediately.
For a mutable view, the first thread submission creates a candidate snapshot,
reconstructs the comparison from that immutable object, and requires its parsed
evidence to match the displayed patch. The verified snapshot is reused by later
threads from that view. Creating a thread protects the selected object with a
private `refs/revia/snapshots/*` ref.

Patch files and stdin have no immutable Git provenance. Revia therefore disables
persistent thread operations for those sources, explains the reason in the UI,
and never creates a Git snapshot for them.

Thread state is repository-local but worktree-shared. It is serialized at
`<git-common-dir>/revia/threads.json` and replaced atomically after each
successful transition.

Resolution, staleness, and escalation remain independent:

- open/resolved is explicit and reversible;
- outdated is presentation metadata and never resolves a concern;
- needs-attention marks a human decision point, so an agent cannot close that
  thread while the flag is set.

Human and agent participants have the same message and thread representation.
Participant kind affects only policies that genuinely distinguish authority,
such as the needs-attention close rule.

## Why

An immutable Git object makes the reviewed code recoverable after the working
tree, branch, or hunk layout changes. It also gives both humans and future agent
participants the same source of truth without requiring a live-edit tracking
system.

The three lifecycle axes answer different questions: whether the concern is
settled, whether its current inline placement is suspect, and whether a human is
required. Combining them would silently turn display uncertainty or escalation
into a semantic resolution decision.

The common Git directory is the stable repository identity across worktrees.
Atomic file replacement ensures the application never publishes a partial JSON
transition.

## Current mechanics

Git acquisition records a resolved object for revision and range input, or the
exact loaded patch evidence for a mutable comparison. Runtime snapshots mutable
evidence with a private alternate index for `Changes`, the index tree for staged,
or `git stash create` for unstaged. It then rebuilds the comparison from that
object and caches it only when the parsed evidence matches. States a single Git
object cannot represent, such as conflicts or dirty-submodule markers, remain
reviewable but reject persistent thread creation with an explicit reason.
`AnchorStore::object` verifies the resulting object and writes a private ref.
`resolve_file` reads the anchored file with `git show`.

`ThreadState` performs transitions purely in memory. `Runtime` clones the current
state, applies one `ThreadOperation`, asks `ThreadRepository` to persist it, and
only then returns the replacement state as an outcome. A failed anchor or write
leaves the application's current state unchanged.

## Rejected alternatives

- Terminal rows or current line numbers are not durable identities; layout and
  context changes invalidate them.
- A Revia-specific mutable "review round" duplicates the immutable snapshots Git
  can already provide.
- Continuous forward tracking is not a correctness requirement. It may improve
  inline placement, but the immutable object remains authoritative.
- Merging outdated into resolved confuses placement quality with concern state.
- Allowing agents to close needs-attention threads removes the human escalation
  point the flag exists to provide.
- Storing state under one checkout would split discussion across worktrees of the
  same repository.
