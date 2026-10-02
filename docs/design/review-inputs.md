# Review Inputs

## Question

Which review inputs does Revia accept, and what comparison does each input mean?

## Rule

Review input has two levels: a source and that source's comparison or input. The
canonical CLI makes both explicit:

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
unstaged tracked changes, and untracked files. `Staged` compares `HEAD` with the
index. `Unstaged` compares the index with the working tree and excludes
untracked files.

`Revision` accepts a commit-ish and displays the patch introduced by its
resolved commit. `A..B` compares the two resolved endpoints. `A...B` compares
their merge base with the resolved right endpoint. The caller's spelling and
the resolved Git objects remain distinct.

Patch file and stdin input contain an already-produced patch. Revia does not
infer a repository or fabricate Git provenance for them.

## Why

The old target flags mixed Git as a provider with several comparison modes.
Source-specific commands keep that boundary explicit, while shortcuts preserve
the shortest common paths without consulting filesystem state or introducing
ambiguous inference.

Using `Changes` consistently in the CLI, domain, and UI gives the default input
one meaning: all current changes relative to `HEAD`.

## Rejected alternatives

- Inferring patch files from positional arguments makes command meaning depend
  on filesystem state and conflicts with valid revision names.
- Retaining `--staged`, `--commit`, and `--range` as peer target flags keeps the
  provider/comparison mixture and creates conflicting combinations.
- Treating a single revision as a working-tree comparison makes the common
  shortcut differ from the canonical revision operation.
- Creating Git snapshots for patch input fabricates provenance the input does
  not possess.

The choice to retain source-specific capture identity is explained in
[`captured-input.md`](captured-input.md). Coherent observation of mutable Git
state is explained in [`mutable-git-input.md`](mutable-git-input.md).
