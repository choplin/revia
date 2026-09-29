# Review Surface

The TUI is one review stream with optional navigation and transient surfaces,
not a collection of independent file viewers. Files, hunks, source rows, and
inline discussions appear in Git order, so direct jumps change the current
target without discarding changeset context.

## Stable regions

The shell gives fixed roles to a header, an optional file rail, the review body,
a current-context row, and a contextual-key row. Status and focus changes update
those regions without changing the body height.

- At 120 columns and wider, the rail takes roughly one quarter of the screen,
  clamped to 28–36 columns.
- From 72 through 119 columns, it takes roughly one third, clamped to 22–28
  columns.
- From 48 through 71 columns, the rail is hidden and the body gets the full
  width.
- Below 48 columns or 8 rows, a stable too-small explanation replaces the shell.

The rail is navigation over the stream, not a file-view switcher. Its file
selection follows the single review cursor. While the rail is focused, a
directory may temporarily hold the list selection so LazyGit-style tree
controls can collapse or expand it without changing the file preview. Sticky
file and hunk rows retain orientation while a scrollbar represents the full
projected stream.

The focused rail follows LazyGit's read-only navigation subset: `j`/`k` move
through visible files and directories, `Enter` opens a file or toggles a
directory, `` ` `` switches between flat and tree layouts, `-`/`=` collapse or
expand all directories, `,`/`.` move by page, and `Home`/`End` or `</>` jump to
the list edges. Mutating working-tree commands are intentionally absent.

## Semantic navigation

The review cursor identifies the selected file, hunk, and optional inline
thread. Focus only determines which region may reinterpret an input. The stream
and file rail are the two persistent focus stops; a thread is a target reached
inside the stream, while composer, help, and rollup are application modes.

Viewport state is derived from a row map but restored through semantic anchors.
Resizing, changing split/stack layout, wrapping, toggling headers or the rail,
and moving through transient modes preserve the same file/hunk-relative reading
position. Direct navigation uses target-sensitive placement; the exact contract
is in [`design/review-navigation.md`](design/review-navigation.md).

## Search and filters

Search indexes file paths, hunk headers, and source lines in Git order. Matches
retain semantic file/hunk/line identity and are re-resolved through the current
row map after relayout. Cancelling restores the invocation cursor, viewport,
focus, and filter. A successful diff reload clears search because its identities
belong to the previous snapshot; a failed reload leaves it intact.

Filters are semantic projections over the loaded diff and persisted thread
state: all changes, needs attention, open threads, or threaded hunks. They do not
delete data or filter rendered cells. Changing a lifecycle value reconciles the
selected target before rebuilding the projection. Full-diff search resets a
restrictive filter for the first result and restores it on cancellation.

## Threads and transient surfaces

Thread cards appear beneath their hunk and keep lifecycle meaning visible through
both text and style. Selected, attention, open, or explicitly expanded cards
show recent content; inactive resolved cards collapse to a provenance-bearing
summary. The rollup orders attention, open, then resolved threads for triage,
while next/previous attention navigation follows Git order in the stream.

The composer is a session-local multiline editor. Submission declares a thread
effect; failure preserves the full draft and target, and success alone clears
and closes it. Help and rollup consume unrelated keys rather than allowing them
to leak into review. Input precedence and mode ownership are described in
[`tui-architecture.md`](tui-architecture.md).

## Presentation principles

- Split and stack are spatial projections of the same source evidence.
- Syntax, diff kind, intraline change, search, selection, and focus are
  independent layers rather than mutually exclusive styles.
- Color is redundant: markers, line numbers, labels, borders, and modifiers
  preserve meaning with `NO_COLOR`.
- Width calculations use terminal cells and grapheme boundaries.
- Hidden source is recoverable through clipping markers or wrapping rather than
  silently discarded.
- Git transport syntax is hidden, while semantic facts such as rename, mode,
  binary, and missing-final-newline state remain visible.

The exact row and responsive-display rules are in
[`design/diff-presentation.md`](design/diff-presentation.md).
