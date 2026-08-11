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
└ context: review stream · status: … · target: src/thread.rs hunk 2/4 ──────┘
```

## Adaptive shell contract

The shell assigns fixed roles to one header line, an optional file-navigation
rail, the review body, one current-context line, and one contextual-key line.
The context and key lines always occupy their own rows, so status updates and
focus changes do not move or resize the review body.

- At 120 columns and wider, the complete header is shown and the rail uses
  roughly one quarter of the screen, clamped to 28–36 columns.
- From 72 through 119 columns, secondary header counts are reduced and the rail
  uses roughly one third of the screen, clamped to 22–28 columns.
- From the supported minimum of 48 columns through 71 columns, the rail is
  hidden and the review body receives the full width.
- Below 48 columns or 8 rows, the stable too-small explanation replaces the
  shell. Empty diffs continue to show the selected target and a recovery action.
- Paths are truncated from the leading side with an ellipsis so the filename
  and nearest parent context survive. Truncation and padding use terminal column
  width rather than Unicode scalar count, including for wide characters.

The visual system has five small semantic roles: added change, removed change,
focus/selection, attention, and muted/resolved. It uses terminal palette colors
without painting a presumed black or white base background. Syntax highlighting
may add non-semantic foreground detail through the terminal's ANSI palette, but
workflow meaning never depends on it or on a fixed dark-theme RGB value. Every
semantic state also has a non-color carrier: `+`/`-` change markers,
`!`/`•`/`✓` lifecycle markers and labels, `›`/`▶` selection markers, and a
double border plus an explicit `◆ STREAM FOCUS` or `◆ THREAD TARGET` label.
With `NO_COLOR` set, semantic and syntax colors are omitted while those
markers, labels, border shapes, boldness, dimming, and reverse emphasis remain.

- The left rail is navigation only. File jumps use `,` / `.` and reveal the
  selected file's first hunk; the rail never reduces the main pane to one-file
  mode.
- The main pane is a single stream in Git diff order. File headers, hunk
  headers, changed lines, and the threads anchored to each hunk are rendered
  in that order. A sticky row keeps the file and hunk containing the viewport
  visible, while a scrollbar represents the complete stream and the current
  visible range.
- The review cursor is the only navigation cursor. The rail follows its current
  file and never claims a separate focus or scrolling target. `j`/`k` and
  arrows always scroll the review stream as they do in Hunk. `[`/`]` traverse
  hunks across file boundaries, `,` / `.` traverse files, `t`/`T` select and
  cycle the current hunk's inline threads, and `}`/`{` jump to the next/previous
  needs-attention thread. `Tab` switches only between the stream and an
  available inline-thread target; when the hunk has no thread, it explains why
  the target cannot change.
- The diff is side-by-side by default on wide terminals and stacked on narrow
  terminals. Each source row has stable old/new line-number gutters derived
  from its parsed Git hunk range. Split mode allocates the two sides evenly
  around an explicit separator; stack mode keeps both numbers beside the
  change marker. Tabs use four-column stops, and long or wide-character lines
  are clipped at grapheme boundaries (or continued with `↪` when stack wrapping
  is enabled). The selected hunk is marked on every row by `┃` as well as a
  high-contrast style, including when hunk headers are hidden.
- Thread actions operate on the visibly selected inline thread. `c` replies,
  `C` starts a separate thread, `x` resolves, `R` reopens, and `a` toggles
  needs-attention. The existing immutable revision/path/hunk anchor remains
  the canonical location.
- The viewport is presentation state, separate from the review cursor. Resizes,
  split/stack changes, wrapping, header visibility, and rail visibility restore
  the same anchored file/hunk-relative position and clamp only at stream edges.
  Reloads preserve the exact file/hunk identity when possible and choose the
  closest hunk in the same file when changed context shifts its coordinates.

## Hunk-compatible keyboard contract

The review surface follows Hunk's command model. Global navigation always acts
on the review stream. File and hunk selection are direct jumps on the same
review cursor rather than a second scrolling model.

| Keys | Behavior |
| --- | --- |
| `j` / `k`, ↑ / ↓ | Scroll one row. |
| `f` / Space, `b` / Shift-Space | Scroll one viewport. |
| `d` / `u` | Scroll half a viewport. |
| `g` / `G`, Home / End | Jump to the start / end of the review stream. |
| `[` / `]` | Jump to previous / next hunk. |
| `,` / `.` | Jump to previous / next changed file. |
| `1` / `2` / `0` | Force split / stack / responsive layout. |
| `s` | Enable or disable the file rail. Below 72 columns an enabled rail remains hidden and reports why. |
| `r` | Reload the current Git diff with the selected context setting. |
| `m` | Show or hide hunk headers. |
| `w` | Toggle line wrapping in stack layout. |
| `=` / `-` | Increase / decrease diff context and reload. Context cannot go below zero. |
| `Tab` / Shift-Tab | Switch between the review stream and an available selected inline thread. |
| `?` | Show this keyboard reference. |
| `q` | Quit from review. In the composer it is text; help and rollup consume it. |
| `Esc` | Close the topmost composer, help, or rollup; from review, quit. |

The only deliberate revia extensions are review actions because Hunk does not
have persistent local review threads: `c` replies when a thread is targeted and
otherwise creates a thread, `C` always creates one, `t` / `T` select or cycle
next/previous inline threads, `x` resolves, `R` reopens, `a` toggles
needs-attention, `o` toggles outdated, `v` opens the rollup, and `{` / `}` jump
among needs-attention threads. Mutating thread actions are available only while
the inline thread is visibly targeted; otherwise the footer explains how to
select one.

From stream focus, the first `t` or `T` makes the cursor's current inline thread
the visible target. Once a thread is targeted, `t` advances and `T` moves back;
both wrap within the current hunk. File and hunk jumps return focus to the stream
and reset the dependent thread target.

## Mode and feedback contract

The two fixed footer rows separate the current target from its available keys.
The context row names the active mode, canonical file/hunk/thread target, and
the latest result. The key row changes among review stream, inline thread,
composer, keyboard help, and thread rollup. Thread mutation hints appear only
when an inline thread is actually available and selected.

| Mode | Navigation and exit |
| --- | --- |
| Review stream | Hunk-compatible stream keys plus dedicated file, hunk, thread, and attention jumps; `Esc` quits. |
| Inline thread | `t` / `T` cycles the visible target, thread verbs act on it, and `Tab` returns to the stream. |
| Composer | `Enter` posts and `Esc` cancels the draft before any quit behavior. |
| Keyboard help | `Esc` or `?` closes help before any quit behavior. |
| Thread rollup | `j` / `k` selects, `Enter` jumps, and `v` or `Esc` returns before any quit behavior. |

Every handled view or mutation action updates the result text. Asynchronous
actions first report that they are pending, then report success or the concrete
failure. Empty diffs, hunks without threads, an empty rollup, zero context, and
thread verbs without a visible thread target are explanatory outcomes rather
than silent no-ops.

At 48–71 columns, the footer uses compact, grapheme-safe target and key labels
so the active target and the mode's primary actions remain visible. Navigation
keys may repeat; modal open/close, submit, and quit keys act on the initial key
press only. Each external operation has an identity, so a duplicate or delayed
result cannot clear or overwrite a newer pending operation.

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
