# Line-based and Structural Diff Rendering

## Question

What role may a future structural or AST-aware diff play relative to Revia's
line-based review evidence?

## Rule

The captured line-based patch and its semantic diff are the canonical review
evidence. A future structural or AST-aware view may exist only as an optional
renderer over that evidence. It must map any selectable structural region back
to an underlying semantic text change or patch fragment.

Parser failure, unsupported languages, binary files, and syntax-version drift
must fall back to the line-based renderer without making the review unavailable.
Structural forward tracking is a presentation improvement, not a requirement
of the diff model.

## Why

Line-based patches are Git-native, accepted directly from file or stdin,
inexpensive to obtain, and identical to the captured evidence used by revision
and changes comparisons.

Structural rendering may reduce whitespace noise or make moves easier to read,
but it introduces language grammars, parse failures, syntax-version drift, and a
second representation that can disagree with Git. Those costs do not justify
making it the source of truth.

## Rejected alternatives

- Replacing patch parsing with structural diff makes unsupported or invalid
  source equivalent to an unavailable review.
- Storing AST coordinates as canonical diff identity couples review meaning to
  parser versions and language support.
- Requiring structural forward tracking for correctness turns a presentation
  improvement into a prerequisite for reviewing an input.

Revisit this decision only with representative diffs showing that the
line-based surface materially obstructs review, an explicit parser/version and
fallback strategy, and a stable mapping back to the semantic diff.
