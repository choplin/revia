# Diff Model

Revia turns an observed diff into a review surface through four distinct
representations:

```text
review request
    |
    v
captured input ---- exact text and resolved source context
    |
    v
parsed patch ------ format-specific syntax
    |
    v
semantic diff ----- reviewer-facing file changes
    |
    v
review presentation
```

The separation reflects how much Revia knows at each point. A request says how
to obtain input. A capture records exactly what was obtained. Parsing explains
the patch syntax. The semantic diff expresses file changes independently of
that syntax. Presentation chooses context, grouping, and layout for the current
surface.

Derived representations carry the captured content identity internally. Their
fields are private and their constructors accept the preceding representation,
so syntax or semantics produced from different captured text cannot be paired
through the public API.

## Capture the exact input

The supported inputs are the five Git comparisons (`Changes`, `Staged`,
`Unstaged`, `Revision`, and `Range`) and the two explicit patch inputs (file and
stdin). Each request maps to a corresponding captured-input variant that owns
the exact patch text and its content hash. Git variants additionally retain the
repository and resolved revision operands; patch-file input retains its path;
stdin remains distinguishable because it cannot be replayed.

For a Git input, the resolved repository context also lets the diff builder
obtain complete file sides for ordinary two-way entries. Mutable source states
are accepted only when regenerating the observed change produces the captured
content identity; a concurrent change causes capture to retry or fail rather
than combining different observations. Patch file and stdin have no source
resolver and therefore provide fragments only.

The request and its result are not independent fields. This prevents a Git
result from being paired with a patch request, a revision from being paired with
another resolved commit, or a patch file from later being reloaded as stdin.
The input contract is in
[`design/review-inputs.md`](design/review-inputs.md). The reason captures retain
source-specific identity is recorded in
[`design/captured-input.md`](design/captured-input.md).

## Parse syntax without making it the domain model

The parser distinguishes plain unified file patches from Git file patches. Git
file bodies distinguish ordinary unified hunks, multi-parent combined hunks,
binary changes, and entries with no content body. Parsed `@@` hunks belong to
this syntax layer; they are not the semantic identity of a change.

The capture remains the lossless record. The parser therefore skips metadata
that Revia does not use and does not carry unknown strings forward in an
`Opaque` node. Malformed structure or an unsupported body form is an explicit
error. The decision to keep parsing as an intermediate boundary is explained in
[`design/patch-parsing.md`](design/patch-parsing.md).

## Represent only the evidence each file provides

The semantic model decides completeness per file. A complete file diff owns
independently obtained file states. A patch-shaped file diff owns only the
sides and fragments declared by its patch.

Ordinary text files from Git comparisons can be complete because Revia can
obtain their source states. Combined Git output remains patch-shaped because it
has multiple before sides. Patch file and stdin input remain patch-shaped even
when a hunk appears to cover the whole file, because Revia did not obtain the
source documents independently.

Missing and empty files are explicit, different states. A new empty file has a
missing before side and a present after side containing an empty document. The
model does not use `Option` to overload missing, unknown, unsupported, and
omitted syntax. The choice to model completeness per file is explained in
[`design/diff-completeness.md`](design/diff-completeness.md).

## Derive presentation last

`ReviewPresentation` is derived from the semantic diff and the current display
options. A complete text diff can be regrouped with a different amount of
context. A patch-shaped diff can only arrange the fragments supplied by the
input; it cannot invent surrounding source or join fragments across an unknown
gap.

A displayed section is therefore a presentation object, not a parsed hunk and
not an identity owned by the diff model. The presentation choices are recorded in
[`design/diff-presentation.md`](design/diff-presentation.md).

## Keep discussion identity outside the diff model

Comments and threads refer to review locations, but they do not define any of
the four diff representations. The ability to comment is also independent of
whether a location can later be persisted. Session locations, durable anchors,
reload behavior, and attachment state belong to the review model and are
decided from the review UX.

## Governing invariants

- Every supported request has exactly one corresponding captured-input shape.
- Captured text and its content hash are created together and cannot disagree.
- Parsed syntax is derived from that capture and cannot be substituted from
  another input.
- Unknown unused metadata stays available in the capture but does not pollute
  the semantic model.
- Complete file diffs never claim content Revia obtained only as patch
  fragments.
- Parsed hunks and displayed sections are not semantic identity owned by the
  diff model.
