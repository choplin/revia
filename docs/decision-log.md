# Decision Log

Newest decisions appear first. Linked documents own the current rule and are
rewritten in place when that rule changes.

| Date | Decision and reason | Supersedes | Current truth |
| --- | --- | --- | --- |
| 2026-09-29 | Replaced the terminal UI stack with native Urushi application and view layers so terminal lifecycle, effects, and physical layout use the same framework while Revia retains semantic state and presentation boundaries. | Ratatui-based physical adapter described by the earlier boundary documents. | [`tui-architecture.md`](tui-architecture.md), [`design/multilayer-elm.md`](design/multilayer-elm.md) |
| 2026-08-14 | Made navigation placement depend on target scale and made relayout restore a semantic viewport anchor, preserving reading context across responsive changes. | Uniform or physical-row-based placement. | [`design/review-navigation.md`](design/review-navigation.md) |
| 2026-08-12 | Established the adaptive single-stream review surface, semantic search and filters, lifecycle-aware inline cards, and a failure-preserving composer so the v1 workflow is usable without changing its anchor or persistence model. | The initial basic review shell. | [`review-surface.md`](review-surface.md), [`design/diff-presentation.md`](design/diff-presentation.md), [`design/review-navigation.md`](design/review-navigation.md) |
| 2026-07-24 | Adopted the Multilayer Elm Architecture: persistent Global and mode slices, explicit binding resolution, pure update, owner-tagged effects/outcomes, and semantic views. | Mode-local programs coordinated by a less explicit Root message/update boundary. | [`tui-architecture.md`](tui-architecture.md), [`design/multilayer-elm.md`](design/multilayer-elm.md) |
| 2026-07-24 | Separated review session, thread lifecycle, diff presentation, application coordination, and terminal concerns so each can be tested without the others. | Procedural coordination that mixed selection, persistence, presentation, and terminal I/O. | [`architecture.md`](architecture.md), [`review-model.md`](review-model.md) |
| 2026-07-23 | Kept Git's line-based patch canonical and deferred structural diff to an optional renderer because universal Git fidelity and stable anchors matter more than language-specific presentation. | Structural diff as a possible primary surface. | [`design/structural-diff.md`](design/structural-diff.md) |
| 2026-07-23 | Modeled thread resolution, outdated placement, and needs-attention escalation as independent axes, with explicit close/reopen and a human-only escalation close rule. | Any single combined lifecycle state. | [`design/anchors-and-threads.md`](design/anchors-and-threads.md) |
| 2026-07-22 | Anchored review discussion to immutable Git objects, including protected snapshots for mutable comparisons, so later edits cannot erase the reviewed evidence. | Live working-tree coordinates as durable identity. | [`review-model.md`](review-model.md), [`design/anchors-and-threads.md`](design/anchors-and-threads.md) |

