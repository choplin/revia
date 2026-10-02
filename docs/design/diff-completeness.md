# Diff Completeness

## Question

Should semantic completeness apply to a whole review input, or independently to
each changed file?

## Decision

Completeness is a property of each file:

```text
FileDiff
  Complete
    Text | Replacement | MetadataOnly
  Patch
    TwoWay | Combined
```

A complete file diff owns independently obtained file states. `Text` owns both
available regular-file text documents and a validated byte-range change map.
`Replacement` covers binary, symlink, and gitlink content as well as transitions
between content kinds.
`MetadataOnly` requires equal content with a changed path or mode.

A patch-shaped file diff owns only the sides, metadata, and fragments declared
by its patch. `TwoWay` has one before and one after side. `Combined` has at least
two ordered parent sides and one result side; every fragment has one range and
prefix column per parent. File absence, empty text, unspecified patch facts, and
unsupported input are distinct states rather than meanings of one `Option`.

Ordinary Git entries become complete only when their source states can be
obtained through the same resolved comparison and validated against the captured
patch identity. Combined output and dirty-submodule markers remain patch-shaped.
Patch-file and stdin entries remain patch-shaped even if a fragment appears to
cover a whole file.

| Input | Ordinary text | Combined | Non-text or kind change | Metadata-only |
| --- | --- | --- | --- | --- |
| Changes / Staged / Unstaged | complete | patch | complete | complete |
| Revision | complete | patch | complete | complete |
| Range | complete | not produced | complete | complete |
| Patch file / stdin | patch | patch | patch | patch |

Constructors validate side cardinality, parent arity, content-kind transitions,
text-range bounds and ordering, unchanged gaps, and no-op states. Consequently,
callers cannot choose a complete variant independently of the evidence it owns.

## Why

One changeset can mix complete ordinary files with combined, binary, gitlink,
or externally supplied fragments. Request-wide completeness would either throw
away available source documents or fabricate documents Revia never observed.

Separating complete and patch-shaped variants lets later phases use richer
operations only where the evidence supports them.

## Rejected alternatives

- Treating every Git result as complete cannot represent combined or dirty
  submodule output faithfully.
- Treating every input as patch-shaped discards available Git source states.
- Optional documents and fragments in one struct admit unsupported combinations.
- Storing hunks in complete text diffs couples semantic changes to one patch
  encoding and context policy.
