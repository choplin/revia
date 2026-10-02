# Captured Input Identity

## Question

Should the result of loading a review input be a source-neutral patch string,
or retain the request-specific facts that produced it?

## Decision

`CapturedInput` retains both the exact patch content and the facts resolved while
loading its source. Its variants follow the supported source families:

- Git captures retain the repository, comparison kind, caller-supplied revision
  spelling, and resolved object IDs applicable to that comparison;
- patch-file captures retain the path; and
- stdin captures remain explicitly one-shot.

Patch content consists of the captured `str` and a hash computed over its exact
bytes. It is not a named wrapper around `str`. Request-specific constructors
create the source facts and content identity together, so a result cannot be
paired with a request of another kind. Derived syntax and semantic values retain
that content identity internally.

Display choices such as context-line count are not capture identity. Git and
patch files may be loaded again from their recorded sources; stdin can only
reuse its original capture. Reloading creates a new capture rather than updating
models derived from the old one.

## Why

A source-neutral result would lose the distinction between immutable Git
objects, mutable Git state, replayable files, and one-shot stdin even when their
patch text happened to match. Independent request and result fields would also
admit combinations that no supported input can produce.

The exact capture remains the lossless record, allowing later parsing to discard
syntax Revia does not use without losing the original evidence.

## Rejected alternatives

- A raw `str` alone has no source identity or reload semantics.
- A separate request plus generic result permits mismatched pairs.
- A bag of optional repository, revision, path, and replay fields makes callers
  reconstruct which combinations are legal.
- An optional `HEAD` for unborn repositories adds a state Revia does not support;
  mutable Git inputs without `HEAD` are rejected.
