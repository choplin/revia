# Review-first TUI v1

## Interaction specification

The screen is a review surface, not a patch viewer with extra commands.

```text
revia  •  Filter: All changes  •  4 files  •  1 need you  •  2 open  •  3 resolved
┌ Files ◆ CURRENT FILE ──────┐╔ Review stream ◆ THREAD TARGET ═════════════════════════════════════════════════════════╗
│› ! src/thread.rs 2h/1t     │║▣ 1/4 src/thread.rs • hunk 2/4                                                          ║
│  • src/anchor.rs 1h/1t     │║▶ @@ -18,3 +18,4 @@                                                                     ║
│  ✓ src/diff.rs   1h/1t     │║┃  18 -old line                                                                         ║
│                            │║┃  18 +new line                                                                         ║
│                            │║  ╰─ ▶ ACTIVE · #007 ! NEEDS ATTENTION · • OPEN · 1 message                             ║
│                            │║  │ human: Please preserve the stable anchor.                                           ║
│                            │║  └─ c reply · x resolve · a attention · o outdated                                     ║
│                            │║                                                                                        ║
│                            │║                                                                                        ║
│                            │║                                                                                        ║
│                            │║                                                                                        ║
│                            │║                                                                                        ║
│                            │║                                                                                        ║
│                            │║                                                                                        ║
│                            │║                                                                                        ║
│                            │║                                                                                        ║
│                            │║                                                                                        ║
│                            │║                                                                                        ║
│                            │║                                                                                        ║
└────────────────────────────┘╚════════════════════════════════════════════════════════════════════════════════════════╝
◆ Context: Inline threads • Target: src/thread.rs • hunk 2/4 • thread #7
Keys: q exit · Tab stream · x resolve · ? help · t/T thread · c reply · C new · a/o flags · e fold
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
The selected search match also carries `⌕`, with `⌕-` and `⌕+` distinguishing
the old and new sides when two matches share one split-layout row.

- The left rail is navigation only. File jumps use `,` / `.` and reveal the
  selected file's first hunk; the rail never reduces the main pane to one-file
  mode.
- The main pane is a single stream in Git diff order. File headers, hunk
  headers, changed lines, and the threads anchored to each hunk are rendered
  in that order. A sticky row keeps the file and hunk containing the viewport
  visible, while a scrollbar represents the complete stream and the current
  visible range.
- The review cursor is the only navigation cursor. The rail can take focus but
  never holds a selection of its own: it always follows the cursor's current
  file. `[`/`]` traverse hunks across file boundaries, `,` / `.` traverse files
  from any focus, `t`/`T` select and cycle the current hunk's inline threads,
  and `}`/`{` jump to the next/previous needs-attention thread.
- Focus is what scopes a key, not a separate mode. `Tab` switches between the
  two regions that hold a lasting position, the stream and the file rail; a
  hidden or too-narrow rail is refused with the reason. An inline thread is a
  target entered with `t`/`T` and left with `Tab`, not a third stop on that
  route.
- Only the keys a focused region claims change meaning: `j`/`k` and arrows
  scroll the stream from the diff and walk files from the rail. Everything
  else — scrolling by page or half page, hunk and file jumps, filters, search —
  acts from wherever focus is and leaves it there. Focus moves only when the
  user moves it, or when the region it rested on stops existing.
- The focused region is the one carrying the focus tone. When the rail takes
  focus its border and selection light up while the diff's selected-hunk box
  and sticky context recede, so a switch is visible on both halves of the
  shell rather than in the footer alone.
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
  needs-attention. `e` keeps a resolved card expanded or lets it fold when it
  becomes inactive. The existing immutable revision/path/hunk anchor remains
  the canonical location.
- Inline cards continue the hunk gutter with `├─` / `╰─` connectors. Their
  lifecycle hierarchy is also textual: `▶ ACTIVE`, `! NEEDS ATTENTION`,
  `• OPEN`, `✓ RESOLVED`, and `~ OUTDATED` remain distinct with `NO_COLOR`.
  Inactive resolved cards use a compact provenance-bearing summary with the
  exact thread ID, closer, and message count, continuing the closer on a narrow
  row when necessary. Selected, attention, open, or explicitly kept cards show
  the latest authored message and applicable actions. Message text
  wraps at grapheme boundaries and is intentionally capped with `…` after two
  rows, or three for the active/attention context.
- The viewport is presentation state, separate from the review cursor. Resizes,
  split/stack changes, wrapping, header visibility, and rail visibility restore
  the same anchored file/hunk-relative position and clamp only at stream edges.
  Reloads preserve the exact file/hunk identity when possible and choose the
  closest hunk in the same file when changed context shifts its coordinates.
- `/` opens incremental plain-text search over file paths, hunk headers, and
  diff-line content. Matching uses a case-insensitive Unicode-lowercase
  substring comparison. Results retain semantic file/hunk/line identities and
  resolve through the current row map, so resize, layout, wrapping, header, and
  rail changes keep the selected match visible without treating a terminal row
  as its identity. Search order is the Git stream order: a file path, then each
  hunk header and its lines, followed by the next file. `n` and `N` move to the
  next and previous match and wrap at both ends.
- Search starts with an empty query and reports both that state and no-match
  results; an empty or no-match query restores the invocation position instead
  of leaving an earlier incremental match selected. `Enter` keeps a
  non-empty search for `n`/`N`; `Esc` while entering a query cancels and restores
  the exact pre-search review cursor and viewport when geometry is unchanged,
  or the same viewport anchor clamped to valid rows after relayout. A successful
  diff reload explicitly clears search because its semantic identities belong
  to the old snapshot; a failed reload leaves search intact.
- The header always names one active review-state filter, including at narrow
  widths, so a reduced stream cannot be mistaken for missing Git changes. `F`
  cycles `All changes`, `Needs attention`, `Open threads`, and `Threaded hunks`;
  `A` returns directly to `All changes`. The filtered views are semantic
  projections over the loaded diff and persisted thread lifecycle state:
  attention/open views include only matching threads and their containing
  hunks, while threaded-hunks includes every thread on each hunk that has one.
  Files, hunks, and threads retain Git diff order within every view.
- A filter with no targets says which filter is active, states that the Git diff
  is still loaded, and gives both recovery keys. Changing a filter keeps the
  closest valid file/hunk/thread identity and reveals it; if a resolve, reopen,
  or flag change removes the selected target, reconciliation happens against
  the replacement persisted state before the next view is built. The raw cursor
  remains available only for later reconciliation: an empty projection exposes
  no footer target and rejects thread composition until a visible target exists.
- Filters survive resize, split/stack, wrapping, header/rail visibility,
  context adjustment, successful or failed reload, and composer/help/rollup
  entry. They are session-local and reset to `All changes` on restart; there is
  no saved preference.
  Full-diff search deliberately does not combine with a filter: the first match
  resets to `All changes` with feedback, cancel restores the invocation filter,
  and choosing `F` or `A` while results are retained clears the search.
- A context reload can change Git's textual hunk header without changing the
  reviewed change. Thread projection therefore resolves an immutable anchor by
  exact path/header first, then by overlap between the anchor's parsed old/new
  ranges and each current hunk's changed-line span in the same path. If a
  context change merges or splits hunks and several candidates overlap, the
  smallest combined old/new hunk start-line distance wins; a tie picks the first
  candidate in Git order. This shared resolver supplies filters,
  inline cards, attention traversal, and rollup landing without changing the
  persisted anchor or thread JSON shape. A non-overlapping anchor remains
  unavailable rather than attaching to an unrelated nearby change.
- `{` and `}` traverse needs-attention threads in Git file/hunk/thread order,
  not lifecycle-priority or creation order. The first jump from a non-attention
  target chooses the first/last item by direction; traversal wraps at both ends.
  A rollup or attention jump that targets a thread hidden by the current filter
  explicitly resets to `All changes`, then lands on and reveals the exact
  file, hunk, and thread in the first review frame.

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
| `/` | Start incremental file-path and diff-content search. While entering, `Enter` keeps the search and `Esc` cancels it. |
| `n` / `N` | Move to the next / previous retained search match, wrapping at either end. |
| `F` / `A` | Cycle review-state filters / return directly to `All changes`. Changing a retained-search view clears search. |
| `1` / `2` / `0` | Force split / stack / responsive layout. |
| `s` | Enable or disable the file rail. Below 72 columns an enabled rail remains hidden and reports why. |
| `r` | Reload the current Git diff with the selected context setting. |
| `m` | Show or hide hunk headers. |
| `w` | Toggle line wrapping in stack layout. |
| `=` / `-` | Increase / decrease diff context and reload. Context cannot go below zero. |
| `Tab` / Shift-Tab | Switch focus between the review stream and the file rail; from a selected inline thread, return to the stream. A hidden or too-narrow rail is refused with the reason. |
| `j` / `k`, arrows (file rail focused) | Move to the next / previous file. Keys the rail does not claim keep their global meaning. |
| `?` | Show the grouped keyboard reference. It marks commands valid in the invoking review, thread, or search-results context. |
| `q` | Quit from review. In the composer it is text; help and rollup consume it. |
| `Esc` | Cancel active search or close the topmost composer, help, or rollup; a non-empty composer requires a second `Esc` before discarding; from review without transient state, quit. |

The only deliberate revia extensions are review actions because Hunk does not
have persistent local review threads: `c` replies when a thread is targeted and
otherwise creates a thread, `C` always creates one, `t` / `T` select or cycle
next/previous inline threads, `x` resolves, `R` reopens, `a` toggles
needs-attention, `o` toggles outdated, `e` keeps/folds resolved context, `v`
opens the rollup, and `{` / `}` jump
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
| Inline thread | `t` / `T` cycles the visible target, thread verbs act on it, and `Tab` returns to the stream. Scrolling does not drop the target. |
| File rail | `j` / `k` and arrows walk files, every other key keeps its global meaning, and `Tab` returns to the stream. Hiding the rail or shrinking below its width hands focus back to the stream. |
| Search input | Character keys edit the incremental query, Backspace deletes, `Enter` keeps a non-empty search, and `Esc` restores the invocation location. |
| Search results | `n` / `N` wraps through matches, `/` starts a new search from the current location, `?` opens context-marked help, and `Esc` cancels back to the invocation location. |
| Composer | `Ctrl-S` posts, `Enter` inserts a newline, and `Esc` cancels before any quit behavior. A non-empty draft requires a second `Esc` to confirm its loss. |
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

### Composer editing contract

The composer is a bounded, growing multiline editor. Its border identifies a
new thread or the exact reply thread, and it displays a terminal cursor at the
current grapheme boundary. It grows to eight editor rows; longer content scrolls
to keep the cursor visible. Narrowing or widening the terminal rewraps the
display without changing the draft.

| Keys | Behavior |
| --- | --- |
| Left / Right | Move by one Unicode grapheme. |
| Up / Down | Move to the closest terminal-cell column on the previous or next explicit line. |
| Home / End | Move to the start or end of the current line. |
| `Alt-Left` / `Alt-Right`, `Ctrl-Left` / `Ctrl-Right` | Move to the previous or next Unicode word. |
| Backspace / Delete | Delete the previous or next Unicode grapheme. |
| `Alt-Backspace` / `Ctrl-Backspace` | Delete the previous word. |
| Enter | Insert a newline. |
| `Ctrl-S` | Post the complete draft. Empty drafts remain open with visible validation. |
| Esc | Close an empty composer. For a non-empty draft, arm discard; press `Esc` again to confirm. Editing after the first press cancels the confirmation. |

While a post is pending, the editor keeps the complete draft and reply target
and does not accept changes that could be lost under the result. A persistence
or anchor failure leaves the composer open with the same draft and target;
`Ctrl-S` retries. Only a successful post clears and closes the composer. Drafts
are intentionally session-local and do not survive restarting revia.

## V1 boundary

Included: Git working-tree, staged, commit, and range diffs; multi-file hunk
reading; syntax-highlighted patch lines; inline persistent review threads;
open/resolved/outdated/needs-attention state; deterministic review rollup;
keyboard-only traversal; review-state filters; case-insensitive plain-text diff
search; and runtime context adjustment.

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
| Keep search identity semantic and snapshot-local | File paths plus hunk/line locations preserve Git order and can be re-resolved after relayout; clearing on reload prevents stale navigation. | Terminal cell or physical row matches break when width, wrapping, or diff content changes. |
| Project filters from semantic review state | The loaded Git diff, immutable anchors, and persisted thread format remain unchanged while navigation can reconcile stable targets before rendering. | Filtering rendered cells would make row coordinates into state and invalidate selection after relayout or lifecycle changes. |
| Use Git order for attention traversal | Review order stays predictable across file boundaries and independent of thread creation or lifecycle-priority rollup order. | Reusing rollup priority order makes next/previous attention depend on unrelated lifecycle grouping. |

## Discussion points

- Does thread author identity need configured display names before multi-user
  storage is introduced, or is the current explicit participant id adequate?
- When an anchored hunk is absent from the currently selected diff target,
  should the rollup offer to open its immutable revision rather than only
  reporting it as unavailable?
