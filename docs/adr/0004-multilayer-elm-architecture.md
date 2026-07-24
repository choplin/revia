# ADR 0004: Adopt the Multilayer Elm Architecture

## Status

Accepted.

## Context

ADR 0003 split the TUI into mode-local Elm programs, but active modes still
owned ephemeral models, shared behavior had no Global module, bindings could not
distinguish delegation from consumption, effect outcomes bypassed Update, and
Ratatui layout was the only view representation.

The Multilayer Elm Architecture requires one persistent Root Model, explicit
Global and Mode slices, a physical-input-to-semantic-event binding layer,
side-effect-free Root Update, effect outcomes returned as semantic events, and
semantic views projected into physical layouts by a framework adapter.

## Concept and responsibility model

| Concept | Invariants and ownership | Representation |
| --- | --- | --- |
| Root Model | Owns Global and every Mode-local state for the complete process lifetime; `active_mode` is only the current interaction context | `app::Model` |
| Global | Owns thread state, status, pending effect, running state, global bindings/update/view/effect vocabulary | `app::global` |
| Review mode | Owns diff request, review session, focus, viewport, layout preferences, review bindings/update/semantic body/effects | `mode::review` |
| Composer mode | Owns persistent input/reply state, modal bindings/update/overlay/effects; entry and exit reset explicitly | `mode::composer` |
| Help mode | Owns modal bindings/update/overlay and an empty effect vocabulary | `mode::help` |
| Rollup mode | Owns persistent selection, modal bindings/update/semantic body/effects | `mode::rollup` |
| Binding resolution | Distinguishes local handling, explicit Global delegation, modal consumption, override, and unbound input | `input::BindingResolution` |
| Effect lifecycle | Update declares user-visible operations, records pending state, and accepts only injected outcomes; adapters perform Git/filesystem/clock work | `app::Effect`, `app::Outcome`, `runtime::Runtime` |
| Semantic view | Describes visible roles, content, feedback, and layout policy without Ratatui types | `semantic` |
| Physical layout | Maps semantic roles and policies to Ratatui widgets, geometry, and styling | `renderer::Renderer` |
| Scenario | Drives the same semantic events, physical inputs, outcomes, viewport, and virtual time used by the application | `app::scenario` tests |

## Decision

- Keep `global`, `review`, `composer`, `help`, and `rollup` models alive in one
  `app::Model`; switch only `ActiveMode`.
- Keep `mode::ActiveMode` as the single closed Mode registry. Represent Root
  Event as an open dispatch protocol implemented by Global and Mode-local Event
  types, rather than another Mode-shaped sum type.
- Normalize Root Effect and Outcome around external operations and carry their
  owner as an `ActiveMode` value. Do not repeat the Mode registry as Effect or
  Outcome variants.
- Treat Mode as the dispatch axis for a complete local program
  (`Model/Event/Bindings/Update/View/Effect`), not as a child of Root Model.
- Normalize Crossterm keys into framework-neutral `PhysicalInput` before binding
  resolution. Each Mode returns `Handle`, `Delegate`, `Consume`, `Override`, or
  `Unbound`; Global bindings run only after `Delegate`.
- Give Global and every Mode its own model, bindings, update, view, and effect
  vocabulary. Mode update/view functions receive their own Model plus
  Mode-owned projected input types and never receive or import Root Model.
- Let Root project `UpdateInput`/`ViewInput`, dispatch the selected Mode
  program, interpret Mode-owned intents, and lift Mode effects. Cross-slice
  transitions exist only in this Root coordination layer.
- Split persisted `ThreadState` from `ThreadRepository`. Semantic Update never
  performs Git, filesystem, transport, or wall-clock work.
- Run declared effects in `runtime::Runtime`, convert every result into an
  owner-tagged `Outcome`, and dispatch it back to that Mode's Update through
  Root.
- Build a framework-independent `semantic::View`, then render it through the
  Ratatui adapter. Screenshots and terminal buffers remain projections.

## Contract and invariant evidence

| Concern | Evidence | Result / limitation |
| --- | --- | --- |
| CLI flags, targets, context defaults, print mode | Existing CLI/diff tests and `main` composition inspection | Preserved |
| Keyboard meanings and modal precedence | Binding-resolution scenarios cover delegation, override, consume, and unbound input | Preserved |
| Review navigation, focus, layout, inline threads | Semantic scenarios and Ratatui TestBackend projection test | Preserved |
| Mode context | Scenario verifies persistent Rollup state and explicit Composer reset | Preserved |
| Effect lifecycle | Scenario verifies pending state and owner-tagged injected outcome re-entry through Mode Update | Preserved |
| Thread JSON shape and atomic replacement | `ThreadState` keeps the same serde shape; repository test reloads persisted state | Preserved |
| Human-only close rule | Pure ThreadState and repository tests | Preserved |
| Scope dependency direction | Mode modules contain no `app`, Root Model, or sibling-Mode references; Root constructs Mode-owned input projections and interprets intents | Preserved |
| Git/domain effects | `runtime::Runtime` is the only new interpreter; Mode/Global Update modules contain no Git or filesystem calls | Preserved |
| Semantic/physical view boundary | `semantic` contains no Ratatui types; renderer consumes only the semantic projection | Preserved |
| Timer/streaming | No semantic timer or stream exists; key-repeat throttling remains a Crossterm adapter concern | Not applicable to semantic core |
| SQL/schema/generated artifacts | Repository contains none | Not applicable |

## Consequences

- The deterministic core can be driven by semantic events and injected outcomes
  without a terminal, filesystem, Git process, or wall clock.
- Each Mode program can be updated and viewed without constructing Root Model;
  Root is orchestration, not an implicit dependency of Mode behavior.
- Mode state retention and cleanup are inspectable Root Model transitions rather
  than consequences of enum construction or destruction.
- Adding an asynchronous runtime later does not change semantic events, Update,
  effects, outcomes, or views; only the effect interpreter changes.
- The architecture introduces more explicit projection types. This is deliberate:
  it prevents Ratatui geometry and transport mechanisms from becoming the
  interaction model.
- Adding a Mode deliberately touches `ActiveMode`, its persistent Root Model
  field, its Root Event protocol implementation, and Root's
  bindings/update/view dispatch. A macro or dynamic registry is deferred until
  that explicit friction becomes a demonstrated maintenance problem.
