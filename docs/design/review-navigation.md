# Review Navigation Viewport Placement

Direct navigation places its target according to the semantic size of the
thing being selected:

- File jumps align the selected file header with the top of the review
  viewport.
- Hunk jumps center a hunk when it fits in the viewport and align its start
  when it does not.
- Point-target jumps, including search matches and thread, attention, and
  rollup targets, move the viewport only as much as needed while preserving a
  two-row `scrolloff`-style margin when space permits.
- Review-stream start and end jumps use the absolute start and end positions.
- Every placement is clamped at document boundaries, and margins shrink for
  viewports too small to preserve them.

The distinction is intentional: large navigation units should expose a clear
reading start, while smaller targets should retain useful surrounding context.
Resize, filter, reload, and presentation changes preserve the semantic viewport
anchor rather than behaving as new navigation jumps.
