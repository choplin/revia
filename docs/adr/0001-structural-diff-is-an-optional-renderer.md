# ADR 0001: Structural diff is an optional renderer, deferred past v1

## Status

Accepted — deferred.

## Decision

v1 renders Git's line-based patch as the canonical review surface. We will not
add a difftastic- or tree-sitter-based structural diff mode in v1.

If a structural mode is added later, it is an **optional renderer** beside the
canonical line-based view. It does not replace Git patch parsing, the commit
anchor, or the thread anchor format.

## Rationale

The current review model anchors a thread to an immutable Git revision, path,
and hunk header. Line-based patches are Git-native, inexpensive, available for
every tracked file, and resolve exactly against that revision. This gives v1 a
single source of truth for display and anchors.

Structural rendering could reduce whitespace noise and make moves easier to
read, but it introduces language grammars, parser failures, syntax-version
drift, and a display that can diverge from the patch Git actually produces.
Treating that rendering as canonical would also make an unavailable parser look
like an unavailable review. Those costs do not earn their keep before the
thread-and-anchor workflow has been used on real reviews.

## Consequences

- The line-based Git patch remains the authoritative way to locate and resolve
  a v1 thread.
- A future structural renderer must be lossy only in presentation: selecting a
  structural region must map back to an existing line-based hunk/anchor before
  a thread is created.
- Parser failure, an unsupported language, or a binary file falls back to the
  existing line-based renderer without changing review state.
- Structural forward tracking is not a correctness requirement. It may improve
  inline placement in a later UI iteration, while the immutable commit anchor
  remains valid regardless.

## Revisit trigger

Reconsider this decision only when dogfooding produces evidence that the
line-based view materially obstructs review — for example, repeated review
threads caused by whitespace-only churn or a recurring need to inspect moves
across a supported language. A proposal must include:

1. one initial language and its parser/version strategy;
2. a measurable rendering benefit on representative diffs;
3. an explicit fallback path to the line-based patch; and
4. proof that thread creation and resolution still use the canonical anchor.
