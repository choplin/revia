# Diff Presentation

## Question

How does Revia present semantic file changes without losing review evidence
when layout, context, or terminal size changes?

## Rule

The normal review surface presents semantic file changes, not patch transport.
Split and stack layouts must expose the same source rows, old/new
line numbers, row kind, syntax, search state, intraline evidence, and clipping
state. Layout may rearrange evidence but may not remove it.

Presentation layers compose in this order:

1. syntax supplies token foregrounds and restrained modifiers;
2. diff kind supplies a quiet row background and stable `+`/`-` marker;
3. intraline replacement supplies a local background and underline;
4. search supplies a local background, reverse modifier, and marker;
5. selected-hunk state supplies header/member markers;
6. focus styles only the owning chrome.

Later layers must preserve earlier foregrounds and compatible modifiers. With
`NO_COLOR`, textual markers, numbers, underline/reverse/bold/dim modifiers, and
border/label differences still carry workflow meaning.

## Parsed facts and hidden transport

The diff pipeline interprets file status, paths, rename/copy similarity, mode
changes, binary state, source coordinates, patch notes, and addition/deletion
counts before presentation. The review stream hides `diff --git`, object-index,
`---`, and `+++` rows. It retains new/deleted, rename/copy, mode, binary,
missing-newline, and range facts in reviewer-facing labels.

The changeset header identifies the comparison, file count, total additions and
deletions, and active filter. File boundaries and rail items retain unique fitted
paths and per-file magnitude. Discussion counts must not displace comparison or
magnitude.

## Source rows

Each source row keeps stable old/new line-number gutters. Complete text diffs
derive them from `TextDocument` line indexes and the semantic change map;
patch-shaped diffs derive them from parsed hunk coordinates. Split gives each
side a complete number/kind/code cell around one divider; stack keeps old and
new numbers beside one code stream. Missing sides remain blank rather than
collapsing a gutter.

Deletion/addition runs may be paired for display. Intraline comparison tokenizes
identifier/number runs, whitespace runs, and punctuation graphemes, computes a
short edit sequence, then refines replacements at grapheme boundaries. Pairing
is presentation evidence only; it does not change source-row identity or claim
that the captured patch supplied a correspondence.

Tabs expand to four-column stops. Clipping and wrapping operate at Unicode
grapheme boundaries and terminal-cell width. A clipped line has a visible edge
marker. Stack wrapping uses continuation rows; split keeps paired sides aligned
to the greater wrapped height.

## Responsive shell

The shell has wide (120+), medium (72–119), and narrow (below 72) policies. The
file rail is absent in narrow mode, while comparison identity, filter, current
target, and recovery actions survive in compact form. Auto layout resolves to
split only when the review body has at least 88 columns; callers may still force
split or stack. Below 48 columns or 8 rows the renderer shows a stable too-small
state rather than attempting a corrupt layout.

The review body owns the majority of the available surface. Sticky context,
footers, and overlays are bounded; optional labels and hints yield before source
evidence and current target identity.

## Why

Reviewers need to compare evidence, not decode Git's wire format. Semantic
metadata retains changes that affect interpretation while removing object IDs
and duplicate path plumbing. Composable styling prevents selection or search
from erasing syntax and change evidence. Non-color carriers make the same state
legible in limited terminals and under `NO_COLOR`.

Keeping split and stack semantically equivalent allows responsive layout without
turning terminal width into a correctness condition. Grapheme/cell-aware fitting
keeps navigation and evidence stable for non-ASCII source.

## Rejected alternatives

- Rendering the raw patch makes transport syntax compete with review evidence.
- Mutually exclusive row styles lose syntax or diff meaning when selection and
  search overlap.
- Hunk-wide reverse paint overwhelms row-kind and syntax evidence; selection is
  carried by hunk markers and bounded chrome.
- Arbitrary byte or scalar clipping can split Unicode and disagree with terminal
  geometry.
- Independent split-side wrapping breaks row correspondence.
- A selected-file-only body hides changeset order and turns the rail into a
  second navigation model.
