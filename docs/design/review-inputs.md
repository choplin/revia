# Review Inputs

Revia primarily reviews comparisons from Git repositories. It also accepts an
existing unified patch from a file or stdin when the review input is already
available as a patch.

## Rule

Review input has two independent levels: a source and that source's comparison
or input. The canonical CLI makes both explicit:

```text
revia git changes
revia git staged
revia git unstaged
revia git revision <REVISION>
revia git range <A..B|A...B>
revia patch <FILE|->
```

Only frequent, unambiguous forms have top-level shortcuts:

```text
revia                 = revia git changes
revia <REVISION>      = revia git revision <REVISION>
revia <A..B|A...B>    = revia git range <RANGE>
revia -               = revia patch -
```

A positional value is never inferred to be a patch file. File input remains
`revia patch <FILE>` whether or not a same-named file currently exists.

`Changes` is the complete current state relative to `HEAD`: staged changes,
unstaged tracked changes, and untracked files. `Revision` accepts a commit-ish
and displays the patch introduced by its resolved commit. Two-dot ranges compare
their endpoints; three-dot ranges compare the merge base with the right endpoint.

Every form normalizes into `DiffSource`, then every acquisition adapter produces
the same `LoadedDiff` containing raw patch text, parsed evidence, and provenance
needed to verify persistent anchors. Raw print, the TUI, reload, and thread
anchoring consume that representation rather than independently choosing what
was reviewed.

## Why

The old target flags mixed Git as a provider with several comparison modes.
That made common invocations hard to remember and made patch input look like a
special Git target. Source-specific commands keep the domain boundary explicit,
while shortcuts preserve the shortest paths without consulting filesystem state
or introducing ambiguous inference.

Keeping the caller's revision spelling in the request makes headers and reload
recognizable. Git resolves revision operands before acquisition, so immutable
comparisons retain their exact target object. Mutable comparisons retain the
loaded patch as verification evidence. Before their first thread is persisted,
Revia creates an immutable candidate snapshot, reconstructs its diff, and
refuses the post unless it represents the displayed evidence. The verified
snapshot is reused for later threads from that loaded view.

Tracked evidence comes from Git's working-tree-aware diff commands, preserving
conflict and dirty-submodule output. `Changes` adds untracked files with one
no-index diff over a private hard-link mirror, including empty files without one
subprocess per path or writing blobs to the repository. Acquisition operates
from the repository root, so starting Revia in a subdirectory does not narrow
the review accidentally.

## Persistence boundary

Git comparisons can establish immutable repository provenance. Patch files and
stdin cannot, so their initial implementation disables persistent thread
operations and explains that boundary in the UI. Patch input never creates a Git
snapshot. Introducing persistence for patch sources requires a separate,
provider-independent immutable provenance design.

Reload repeats the normalized request. A patch file is reread; stdin reuses the
initially loaded patch because the stream is not replayable.

## Rejected alternatives

- Inferring patch files from positional arguments makes command meaning depend
  on filesystem state and conflicts with valid revision names.
- Retaining `--staged`, `--commit`, and `--range` as peer target flags keeps the
  provider/comparison mixture and creates conflicting combinations.
- Treating a single revision as a working-tree comparison makes the common
  shortcut differ from the canonical revision operation.
- Creating Git snapshots for patch input fabricates provenance the input does
  not possess.
