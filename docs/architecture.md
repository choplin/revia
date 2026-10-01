# Architecture

This is the starting point for developers changing Revia. Revia is an
interactive terminal UI for reviewing Git diffs. Git repository comparisons
are its primary input; an existing unified patch can also be reviewed from a
file or stdin. Revia lets a reviewer navigate either input as one ordered
changeset. Git sources can store discussions against immutable Git-backed hunk
anchors. Persistent discussions are unavailable for patch input because it has
no immutable repository provenance. Revia does not stage, edit, or commit
tracked content.

Three documents expand the main architectural units:

- [`review-model.md`](review-model.md) explains parsed diffs, immutable anchors,
  thread state, persistence, and projection onto a current diff.
- [`review-surface.md`](review-surface.md) explains the single review stream,
  semantic navigation, filtering, searching, and responsive presentation.
- [`tui-architecture.md`](tui-architecture.md) explains the Multilayer Elm
  application, effect boundary, semantic view, and Urushi runtime adapter.
- [`design/review-inputs.md`](design/review-inputs.md) defines source-specific
  commands, shortcut normalization, and acquisition boundaries.

Precise rules and their rationale live under [`design/`](design/). The
[`decision log`](decision-log.md) records when those rules changed without
duplicating their current definitions.

## System flow

```text
CLI arguments
    |
    v
DiffRequest -- source adapter --> LoadedDiff --> app::Model
                  |                 ^
                  +-- Git/patch ----+
                                             /            \
physical input / surface -------------------/              \
                                                            v
                                              mode-local update + Root
                                                     |          |
                                                     |          +--> app::Effect
                                                     |                  |
                                                     |              Runtime
                                                     |                  |
                                                     +<-- Outcome <-----+
                                                     |
                                                     v
                                              semantic::View
                                                     |
                                                     v
                                              UrushiRenderer
                                                     |
                                                     v
                                                urushi::View
```

`main` composes the application. `adapter::cli` turns Clap arguments into a
renderer-independent `DiffRequest`; `adapter::diff` routes acquisition to Git,
a patch file, or stdin; and `domain::diff` parses the resulting patch into
`LoadedDiff`. When `--print` is used or stdout is not a terminal, `main` writes
the raw loaded patch and does not open the review store or TUI.

The interactive path opens `Runtime`, builds the persistent Root model, and
hands both to `adapter::terminal::ReviaApplication`. Urushi owns terminal input,
surface updates, effect execution, drawing, and session restoration. Revia owns
the meaning of input, state transitions, external operations, and the semantic
screen it wants rendered.

## Source layout

The first directory below `src/` identifies why code changes:

```text
src/
  main.rs          composition root and process-level error context
  domain/          diff, anchor, thread, and review-session concepts
  app/             deterministic Multilayer Elm state, modes, input, and view
    root.rs        Root model, dispatch, coordination, and view composition
  presentation/    semantic-view to styled-text transformations
  adapter/
    diff.rs         source router for Git, patch files, and stdin
    git/            Git commands, thread persistence, and wall-clock access
    terminal/       Urushi application and physical view construction
    cli.rs          Clap argument transport
    runtime.rs      app::Effect interpreter
```

Adapters consume `app`, `presentation`, and `domain`; the application consumes
`domain` and emits semantic `app::view` values; presentation consumes that
semantic view and domain evidence. One deliberate pure dependency crosses back:
the review mode uses `presentation::ReviewRowMap` because viewport navigation
must follow the rows produced by wrapping and layout. Domain modules do not
import application, presentation, terminal, filesystem, process, or clock APIs.
`main` is the only composition root.

## Responsibility boundaries

| Area | Owner | Boundary |
| --- | --- | --- |
| CLI transport | `adapter::cli` | Normalizes canonical source commands and shortcuts into one request; it does not load data. |
| Diff acquisition | `adapter::diff` and `adapter::git::diff` | Routes by source, owns Git's command-line representation and patch reads, and returns one common loaded representation. |
| Review identity and lifecycle | `domain::{anchor, review, thread}` | Owns immutable locations, current selection, and thread transitions without I/O or rendering concerns. |
| Diff parsing | `domain::diff` | Produces files, hunks, lines, file-change facts, and magnitude from patch text. |
| Deterministic application | `app` and `app::mode` | Resolves input, updates persistent state, coordinates cross-slice intents, and declares effects without performing I/O. |
| External operations | `adapter::runtime` and `adapter::git` | Runs Git, filesystem, persistence, and clock operations and returns typed outcomes. |
| Semantic screen | `app::view` and mode `view` functions | Describes visible roles and content without Urushi layout types. |
| Diff-row presentation | `presentation` | Converts semantic diff evidence into width-bounded styled rows; display rows are never review identity. |
| Physical layout | `adapter::terminal` | Maps the semantic screen to Urushi and adapts terminal input/effects to the application. |
| Responsive navigation state | `app::view_state` | Owns focus, viewport position, layout preference, terminal-width classes, and width-safe fitting independent of the backend. |

Dependencies point from outer adapters toward these contracts. Domain state does
not depend on Urushi, terminal geometry, Git process APIs, or filesystem APIs.
Mode programs do not import the Root model or sibling modes. The Root layer is
the only place that coordinates slices and changes the active mode.

## Side effects and repository writes

"Read-only" means Revia never changes the reviewed working tree, index, or
commits. Review metadata still has to be durable:

- diffs and committed content are read through Git subprocesses;
- immutable Git comparisons retain their resolved target object; mutable views
  snapshot on first thread submission and accept that object only when its
  reconstructed diff matches the displayed evidence, then reuse and protect it
  with a `refs/revia/snapshots/*` ref;
- thread state is atomically replaced at
  `<git-common-dir>/revia/threads.json`.

Patch files and stdin do not open this store or create Git snapshots. Their TUI
keeps reload and navigation available, but rejects persistent thread operations
with the missing-provenance reason.

Using the common Git directory makes review state and protected snapshots shared
by all worktrees of the same repository. The precise anchor and lifecycle rules
are defined in [`design/anchors-and-threads.md`](design/anchors-and-threads.md).

## Architectural invariants

- Git's line-based patch remains the authoritative review evidence. Structural
  rendering may only be an optional projection; see
  [`design/structural-diff.md`](design/structural-diff.md).
- A review target is semantic (`path` plus hunk identity), never a terminal row
  or cell coordinate.
- Persisted anchors are immutable. Projection onto a reloaded diff is derived
  and must not rewrite the anchor.
- Thread resolution, staleness, and human attention are independent axes.
- The deterministic update path performs no Git, filesystem, terminal, or wall
  clock work.
- Physical layout consumes a semantic view; Urushi types do not define the
  interaction model.
- At most one external operation is pending. Results carry operation identity
  and ownership so stale completions cannot replace newer state.
