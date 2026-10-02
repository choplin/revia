# Coherent Mutable Git Input

## Question

How can Revia enrich mutable Git changes with complete file states without
presenting data assembled from different moments?

## Decision

`Changes`, `Staged`, and `Unstaged` are captured from the repository root and do
not mutate the repository. `Changes` includes staged, unstaged, and untracked
files relative to `HEAD`; `Staged` compares `HEAD` with the index; `Unstaged`
compares the index with tracked working-tree files. Repositories without `HEAD`
are unsupported.

The captured patch is the observation boundary. When semantic interpretation
also reads complete file sides, the adapter regenerates the comparison and
accepts those sides only if it reproduces the captured content identity. A
concurrent change causes retry or failure. Conflict output, dirty-submodule
markers, and other entries without one complete two-sided state remain
patch-shaped.

Untracked files are included without writing blobs or updating the real index.
The concrete batching and temporary-storage strategy is an adapter
implementation detail as long as it preserves that boundary.

## Why

Reading a patch and then independently reading mutable files can combine states
that never existed together. Verification provides coherent evidence without
requiring edit history, CRDT state, or mutation of the user's repository.

## Rejected alternatives

- Accepting independently read sides without verification permits torn views.
- Treating `Changes` as unstaged-only omits staged and untracked work.
- Running from the invocation directory silently narrows the review.
- Writing to the real index or object database violates the read-only contract.
