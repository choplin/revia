# Immutable Anchors and Thread Lifecycle

## Rule

Every thread is stored against an immutable Git revision, repository path, and
hunk header. Committed comparisons resolve the requested commit. Threads created
from working-tree, staged, or range views first create an immutable stash commit
and protect it with a private `refs/revia/snapshots/*` ref.

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

`AnchorStore::committed` verifies and canonicalizes a commit. For other diff
targets, `snapshot_working_tree` runs `git stash create`, rejects an empty
snapshot, and writes a private ref to protect the returned object from garbage
collection. `resolve_file` reads the anchored file with `git show`.

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
  inline placement, but the immutable revision remains authoritative.
- Merging outdated into resolved confuses placement quality with concern state.
- Allowing agents to close needs-attention threads removes the human escalation
  point the flag exists to provide.
- Storing state under one checkout would split discussion across worktrees of the
  same repository.

