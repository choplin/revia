# Review Model

The review model connects Git's patch structure to durable discussion without
making terminal rows into identity. It has three layers: the loaded comparison,
an immutable anchor for each discussion, and a derived projection that places
those anchors in the current comparison when possible.

## Comparisons and parsed evidence

`DiffRequest` pairs a `DiffTarget` with a context-line count. Targets cover the
working tree, index, one commit, and a revision range. `LoadedDiff::load` invokes
`git diff` or `git show`, retains the raw patch for print mode, and parses a
`DiffDocument` for the TUI.

The parsed document preserves only review-relevant facts:

- files in Git order, including previous paths and new/deleted state;
- rename/copy similarity, mode changes, and binary status;
- hunk headers and parsed old/new ranges;
- source rows classified as context, addition, deletion, or patch note; and
- per-file and whole-comparison addition/deletion counts.

Git transport rows such as object IDs are not review identity and are not shown
as ordinary source rows. The detailed presentation rule is in
[`design/diff-presentation.md`](design/diff-presentation.md).

## Location, anchor, and current selection

`HunkLocation` is the location used by navigation and projection: one repository
path plus one hunk header. `Anchor` adds an immutable Git revision to that
location. The revision is the source of truth for the reviewed code even when
the current working tree later changes.

`ReviewSession` owns the current `LoadedDiff` and one `ReviewCursor`. The cursor
selects a file, a hunk, and an inline thread index. Its transitions preserve the
dependency between those values: selecting a file resets hunk and thread;
selecting a hunk resets thread. Replacing a diff restores the same file and exact
hunk when possible, then chooses the nearest hunk start in that file before
falling back to clamped indices.

The viewport is intentionally separate. Scrolling, wrapping, responsive layout,
and terminal rows belong to review-mode presentation state, so relayout cannot
change the selected review target.

## Threads and lifecycle

`ThreadState` is the serializable domain aggregate. It owns monotonic opaque
`ThreadId` values, messages, participants, and lifecycle transitions without a
Git or filesystem dependency. Human and agent authors use the same `Participant`
shape.

The lifecycle has three orthogonal properties:

- `Resolution` is open or resolved and may be reopened;
- `outdated` is a display hint about the current placement, not resolution;
- `needs_attention` is a human escalation flag. An agent cannot close a thread
  while this flag is set.

`ThreadRepository` is the persistence adapter. Each successful operation clones
the current state, applies one domain transition, writes pretty JSON to a
temporary file, and renames it over the store. The in-memory state is replaced
only after persistence succeeds.

## Projecting immutable anchors

The persisted anchor never follows a changing diff. Review mode derives a
current-hunk projection for display and navigation:

1. match the same path and exact hunk header;
2. otherwise, parse the anchor's old/new ranges and find same-path hunks whose
   changed spans overlap;
3. when several hunks overlap, prefer the smallest combined old/new start-line
   distance, then the first candidate in Git order;
4. if none overlaps, leave the anchor unavailable rather than attaching it to a
   nearby but unrelated change.

Filters, inline cards, attention traversal, and rollup landing all use this
shared projection. The fallback exists because changing context can split,
merge, or rename textual hunk headers without changing the reviewed lines. It
does not claim to track edits semantically across revisions.

## Change map

- Change Git invocation in `adapter::git::diff`; change patch parsing in
  `domain::diff`, then verify adapter and parser tests together.
- Change selection identity or replacement behavior in `domain::review` and the
  review-mode scenarios; do not encode it in rendered rows.
- Change lifecycle rules in `ThreadState`, then adapt `Runtime` only for the
  operation and persistence mechanics.
- Change anchor creation or retrieval in `adapter::git::anchor::AnchorStore`;
  preserve immutable Git provenance and the worktree-shared storage boundary.
- Change current-diff placement in the review mode's projection helpers; never
  mutate persisted anchors to make display easier.
