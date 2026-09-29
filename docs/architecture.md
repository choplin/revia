# Architecture

This is the starting point for developers changing Revia. Revia is a terminal,
Git-only review application: it loads a line-based diff, lets a reviewer navigate
one ordered changeset, and stores discussions against immutable Git-backed hunk
anchors. It does not stage, edit, or commit tracked content.

Three documents expand the main architectural units:

- [`review-model.md`](review-model.md) explains parsed diffs, immutable anchors,
  thread state, persistence, and projection onto a current diff.
- [`review-surface.md`](review-surface.md) explains the single review stream,
  semantic navigation, filtering, searching, and responsive presentation.
- [`tui-architecture.md`](tui-architecture.md) explains the Multilayer Elm
  application, effect boundary, semantic view, and Urushi runtime adapter.

Precise rules and their rationale live under [`design/`](design/). The
[`decision log`](decision-log.md) records when those rules changed without
duplicating their current definitions.

## System flow

```text
CLI arguments
    |
    v
DiffRequest -- git diff/show --> LoadedDiff --> app::Model
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

`main` composes the application. `cli` turns Clap arguments into a
renderer-independent `DiffRequest`; `diff` invokes Git and parses the resulting
patch into `LoadedDiff`. When `--print` is used or stdout is not a terminal,
`main` writes Git's raw patch and does not open the review store or TUI.

The interactive path opens `Runtime`, builds the persistent Root model, and
hands both to `tui_app::ReviaApplication`. Urushi owns terminal input, surface
updates, effect execution, drawing, and session restoration. Revia owns the
meaning of input, state transitions, external operations, and the semantic
screen it wants rendered.

## Responsibility boundaries

| Area | Owner | Boundary |
| --- | --- | --- |
| CLI transport | `cli` | Selects a repository, diff target, context, and print/TUI path; it does not load Git data. |
| Git patch acquisition and parsing | `diff` | Produces semantic files, hunks, lines, file-change facts, and magnitude without terminal state. |
| Review identity and lifecycle | `anchor`, `review`, `thread` | Own immutable locations, current selection, thread transitions, and durable thread data without rendering concerns. |
| Deterministic application | `app`, `mode`, `input` | Resolves input, updates persistent state, coordinates cross-slice intents, and declares effects without performing I/O. |
| External operations | `runtime` | Runs Git and filesystem operations declared by the application and returns typed outcomes. |
| Semantic screen | `semantic`, mode `view` functions | Describes visible roles and content without Urushi layout types. |
| Diff-row presentation | `presentation`, `syntax`, `styled_text`, `renderer` | Converts semantic diff evidence into width-bounded styled rows; display rows are never review identity. |
| Physical layout | `urushi_renderer`, `tui_app` | Maps the semantic screen to Urushi layout and adapts Urushi input/effects to the application. |
| Responsive navigation state | `ui` | Owns focus, viewport position, layout preference, terminal-width classes, and width-safe fitting independent of the backend. |

Dependencies point from outer adapters toward these contracts. Domain state does
not depend on Urushi, terminal geometry, Git process APIs, or filesystem APIs.
Mode programs do not import the Root model or sibling modes. The Root layer is
the only place that coordinates slices and changes the active mode.

## Side effects and repository writes

"Read-only" means Revia never changes the reviewed working tree, index, or
commits. Review metadata still has to be durable:

- diffs and committed content are read through Git subprocesses;
- a thread created against working-tree, staged, or range output receives an
  immutable stash commit and a protecting `refs/revia/snapshots/*` ref;
- thread state is atomically replaced at
  `<git-common-dir>/revia/threads.json`.

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

