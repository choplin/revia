# Line-based and Structural Diff Rendering

## Rule

Git's line-based patch is the canonical review surface and anchor source. A
future structural or AST-aware view may exist only as an optional renderer over
that evidence. It must map any selectable structural region back to an existing
line-based hunk before a thread is created.

Parser failure, unsupported languages, binary files, and syntax-version drift
must fall back to the line-based renderer without making the review or its
threads unavailable. Structural forward tracking is a presentation improvement,
not a correctness requirement for anchors.

## Why

Line-based patches are Git-native, available for every tracked file, inexpensive
to obtain, and identical to the evidence used by revision and changes
comparisons. The immutable revision/path/hunk anchor can always recover that
evidence.

Structural rendering may reduce whitespace noise or make moves easier to read,
but it introduces language grammars, parse failures, syntax-version drift, and a
second representation that can disagree with Git. Those costs do not justify
making it the source of truth.

## Rejected alternatives

- Replacing patch parsing with structural diff makes unsupported or invalid
  source equivalent to an unavailable review.
- Storing AST coordinates as the canonical anchor couples persisted discussion
  to parser versions and language support.
- Requiring structural forward tracking for correctness solves a display
  placement problem by weakening immutable provenance.

Revisit this decision only with representative diffs showing that the line-based
surface materially obstructs review, an explicit parser/version and fallback
strategy, and proof that thread identity remains line-based.
