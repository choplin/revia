# Review-first TUI v1

## Interaction specification

The screen is a review surface, not a patch viewer with extra commands.

```text
┌ revia · 4 files · 1 needs attention · 2 open · 3 resolved ───────────────┐
│ Files / review state       │ Review stream                              │
│ ! src/thread.rs        2   │ src/thread.rs                              │
│ · src/anchor.rs        1   │ @@ -…                                     │
│ ✓ src/diff.rs          1   │ - old line                                │
│                           │ + new line                                │
│                           │ ┌ OPEN · human · thread #7 ──────────────┐ │
│                           │ │ Please preserve …                       │ │
│                           │ └─────────────────────────────────────────┘ │
└ focus: review · Tab focus · [/] hunk · t thread · } attention · c reply ┘
```

- The left rail is navigation only. File jumps use `,` / `.` and reveal the
  selected file's first hunk; the rail never reduces the main pane to one-file
  mode.
- The main pane is a single stream in Git diff order. File headers, hunk
  headers, changed lines, and the threads anchored to each hunk are rendered
  in that order.
- `Tab` cycles the visible focus indicator between files, review, and threads,
  but `j`/`k` and arrows always scroll the review stream as they do in Hunk.
  `[`/`]` traverse hunks across file boundaries, `,` / `.` traverse files,
  `t` selects the current hunk's thread list, and `}`/`{` jump to the
  next/previous needs-attention thread.
- The diff is side-by-side by default on wide terminals and stacked on narrow
  terminals. `s` explicitly switches between the two layouts. The selected
  hunk uses a high-contrast region in either layout.
- Thread actions operate on the visibly selected inline thread. `c` replies,
  `C` starts a separate thread, `x` resolves, `R` reopens, and `a` toggles
  needs-attention. The existing immutable revision/path/hunk anchor remains
  the canonical location.

## Hunk-compatible keyboard contract

The review surface follows Hunk's command model. Global navigation always acts
on the review stream, regardless of which rail is visibly focused; file and
hunk selection are direct jumps rather than a second scrolling model.

| Keys | Behavior |
| --- | --- |
| `j` / `k`, ↑ / ↓ | Scroll one row. |
| `f` / Space, `b` / Shift-Space | Scroll one viewport. |
| `d` / `u` | Scroll half a viewport. |
| `g` / `G`, Home / End | Jump to the start / end of the review stream. |
| `[` / `]` | Jump to previous / next hunk. |
| `,` / `.` | Jump to previous / next changed file. |
| `1` / `2` / `0` | Force split / stack / responsive layout. |
| `s` | Show or hide the file rail. |
| `r` | Reload the current Git diff with the selected context setting. |
| `m` | Show or hide hunk headers. |
| `w` | Toggle line wrapping in stack layout. |
| `Tab` | Cycle the visible focus affordance; it never changes what scrolling means. |
| `?` | Show this keyboard reference. |

The only deliberate revia extensions are review actions because Hunk does not
have persistent local review threads: `c` creates/replies through the inline
composer, `t` selects an inline thread, `x` resolves, `R` reopens, `a` toggles
needs-attention, and `{` / `}` jump among needs-attention threads. These are
scoped to the selected inline thread and never replace the global Hunk
navigation commands above.

## V1 boundary

Included: Git working-tree, staged, commit, and range diffs; multi-file hunk
reading; syntax-highlighted patch lines; inline persistent review threads;
open/resolved/outdated/needs-attention state; deterministic review rollup;
keyboard-only traversal; and runtime context adjustment.

Excluded deliberately: staging or patch editing, line-level Git mutations,
remote pull-request providers, prose/LLM summaries, non-Git VCS support,
and a structural-diff renderer. These exclusions keep the product read-only
and leave Git's patch plus immutable anchors as the source of truth.

## Decision log

| Decision | Rationale | Rejected alternative |
| --- | --- | --- |
| Render one continuous review stream | A reviewer retains the order and context of the complete changeset while navigating from the rail. | Rendering only the selected file turns the rail into a view switcher and hides review flow. |
| Place threads beneath their hunk | The discussion, lifecycle state, and the changed code are readable together. | A rollup-only or modal CRUD view loses the code context. |
| Keep the current file/hunk selection as the sole navigation model | File, hunk, thread, and attention jumps can all resolve to the same canonical anchor and visible location. | Separate sidebar and diff selections can drift and make the current target ambiguous. |
| Preserve the existing anchor and JSON thread store | They already provide immutable Git revision provenance and restart persistence. | Display row coordinates are not stable review identities. |

## Discussion points

- Should a future version add a compact filter for files with attention/open
  threads, once large real-world changesets establish the needed vocabulary?
- Does thread author identity need configured display names before multi-user
  storage is introduced, or is the current explicit participant id adequate?
- When an anchored hunk is absent from the currently selected diff target,
  should the rollup offer to open its immutable revision rather than only
  reporting it as unavailable?
