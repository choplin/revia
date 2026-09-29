# Review Navigation and Viewport Restoration

## Rule

The review cursor is the only file/hunk selection model. The file rail follows
it, hunk and file jumps update it, and inline thread selection extends it. The
focused rail may hold a transient directory target for tree navigation; this
does not change the review cursor or the previewed file. Focus scopes keys to
the stream, rail, or selected thread.

Direct navigation places a target according to its semantic size:

- file jumps align the file header with the top of the viewport;
- hunk jumps center a hunk when it fits and align its start when it does not;
- point targets (search matches, threads, attention items, and rollup landings)
  move only as far as needed while preserving a two-row margin when possible;
- stream start/end jumps use the absolute boundaries;
- all placements clamp at document boundaries, and margins shrink in small
  viewports.

Relayout is not navigation. Before resize, split/stack change, wrapping, hunk
header visibility, rail visibility, or another geometry change, review mode
captures a semantic `ViewportAnchor`: stream start, a file path, or a hunk
location plus an offset and extent. It rebuilds the row map, resolves that
anchor, and restores the reading position. End-relative scrolling remains
end-relative when content height changes.

Search cancellation stores and restores the invocation cursor, focus, viewport
anchor, end-relative position, geometry, and filter. If geometry changed while
search was open, restoration resolves the semantic anchor into the new row map
and clamps it.

## Why

Files and hunks are reading units; revealing their start gives a stable entry
point. Search hits and threads are points inside an existing reading context, so
minimal movement preserves useful surrounding evidence. Treating resize or
wrapping as a fresh jump makes the reviewer's position drift even though their
intent did not change.

One selection model prevents the rail, body, footer target, and thread actions
from disagreeing. Semantic anchors survive row-count changes that invalidate
physical coordinates.

## Edge cases

- A hidden or narrow-screen rail cannot retain focus; review mode returns focus
  to the stream.
- A file with no hunks still has a file-header target and does not break hunk
  traversal across neighboring files.
- Changing a semantic filter reconciles to the closest visible file/hunk/thread;
  an empty projection exposes no hidden target.
- A successful reload keeps exact file/hunk identity where possible, otherwise
  uses the closest same-file hunk and clamps. It clears snapshot-local search.
- Thread or attention landing resets a filter to All changes when necessary to
  make the requested target visible.

## Rejected alternatives

- Keeping separate file selections in the rail and stream permits them to drift
  and makes thread actions ambiguous. A transient directory target is safe
  because it cannot own a hunk or receive a review action.
- Restoring a raw row number after relayout points at different content whenever
  wrapping, cards, filters, or terminal width changes row counts.
- Centering every target wastes context for large files and hunks; top-aligning
  every point target discards useful preceding code.
- Recomputing the target from rendered text makes styling and layout part of
  domain identity.
