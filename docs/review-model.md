# Review Model

The review model connects a diff presentation to durable discussion without
making terminal rows into identity. It has three layers: the current review
presentation, an immutable anchor for each discussion, and a derived projection
that places those anchors in the current presentation when possible.

The diff representations consumed by this model are explained separately in
[`diff-model.md`](diff-model.md).

## Location, anchor, and current selection

`HunkLocation` is the location used by navigation and projection: one repository
path plus one hunk header. `Anchor` adds an immutable Git object ID to that
location. The object is the source of truth for the reviewed code even when
the current working tree later changes.

`ReviewSession` owns the current `ReviewPresentation` and one `ReviewCursor`.
The presentation is derived from `ReviewDiff`; neither parsed patch syntax nor
terminal rows become session identity. The cursor selects a file, a displayed
section, and an inline thread index. Its transitions preserve the dependency
between those values, and replacing a presentation restores the closest
defensible semantic selection rather than a physical row. The exact restoration
and navigation rules are defined in
[`design/review-navigation.md`](design/review-navigation.md).

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

`ThreadRepository` is the persistence adapter. Domain transitions remain
independent of storage, and the in-memory state changes only after durable
persistence succeeds. The storage and lifecycle details are defined in
[`design/anchors-and-threads.md`](design/anchors-and-threads.md).

## Projecting immutable anchors

The persisted anchor never follows a changing diff. Review mode derives a
current placement for display and navigation, and every consumer uses that
shared projection rather than implementing its own matching. If no defensible
placement exists, the anchor remains unavailable instead of attaching to nearby
but unrelated content. The current matching rule is defined in
[`design/anchors-and-threads.md`](design/anchors-and-threads.md).

## Change map

- Change selection identity or replacement behavior in `domain::review` and the
  review-mode scenarios; do not encode it in rendered rows.
- Change lifecycle rules in `ThreadState`, then adapt `Runtime` only for the
  operation and persistence mechanics.
- Change anchor creation or retrieval in `adapter::git::anchor::AnchorStore`;
  preserve immutable Git provenance and the worktree-shared storage boundary.
- Change current-diff placement in the review mode's projection helpers; never
  mutate persisted anchors to make display easier.
