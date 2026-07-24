# ADR 0003: Give each interaction mode its own Elm program

## Status

Superseded by ADR 0004.

## Context

The first Elm refactor introduced a root `Model`, `Message`, `update`, and
read-only `view`, but the root message enum and update function still knew every
review, composer, help, and rollup transition. That made modal behavior explicit
but not locally owned: adding a composer key or rollup transition required
editing the root program.

## Concept model

| Concept | Valid states and invariants | Owned transitions | Representation |
| --- | --- | --- | --- |
| Review mode | Focus, scroll, layout, rail, hunk-header, wrap, and viewport state remain valid while overlays are open | Navigation and display settings update locally; Git, persistence, and mode changes become `review::Effect` values | `mode::review::Model`, `Message`, `Effect`, `update`, and `view` |
| Composer mode | Input and optional reply target exist only while composing | Insert/delete are local; cancel and submit become `composer::Effect` values | `mode::composer::Model`, `Message`, `Effect`, `update`, and `view` |
| Help mode | No mutable local data is required | Close becomes a `help::Effect` | `mode::help::Model`, `Message`, `Effect`, `update`, and `view` |
| Rollup mode | Selection is clamped by transitions to the current ordered thread IDs and survives closing/reopening | Move is local; close and open-thread become `rollup::Effect` values | `mode::rollup::Model`, `Message`, `Effect`, `update`, and `view` |
| Root application | Exactly one interaction mode receives keyboard messages; review state remains available beneath overlays | Routes messages, interprets mode effects, performs Git/filesystem work, and changes modes | `main::Model`, root `update`, and `perform_*_effect` |

## Decision

- Define `Model`, `Message`, `Effect`, `update`, `message_for_key`, and `view`
  together in each mode module.
- Keep review-session, thread-store, syntax resources, and status in the root
  application because they are shared data or external adapters rather than
  transient mode state.
- Let mode updates return effect values instead of performing Git reads,
  filesystem writes, thread persistence, or cross-mode transitions.
- Interpret effects only at the root composition boundary.
- Keep the review model alive under composer/help/rollup rendering. Preserve the
  rollup model while inactive so its selection behavior does not change.

## Contract and invariant evidence

| Concern | Evidence | Result / limitation |
| --- | --- | --- |
| CLI flags and print mode | `main` composition and existing CLI/diff tests | Preserved |
| Keyboard mapping and overlay precedence | Mode-local key maps plus existing global-key and composer tests | Preserved |
| Split/stack rendering and inline threads | Existing TestBackend fixture | Preserved |
| Thread JSON and atomic persistence | `thread` adapter is unchanged; existing persistence tests pass | Preserved |
| Review selection and diff reload | Existing review tests and root effect inspection | Preserved |
| Mode transitions and effect boundaries | New unit tests for all four mode updates | Preserved |
| SQL/schema/generated artifacts | Repository contains none | Not applicable |

## Consequences

- Each interaction mode can evolve and be tested without extending one global
  update match.
- The root still coordinates effects that need shared domain state or external
  adapters. This is deliberate; moving the thread store into a UI mode would
  invert the dependency boundary.
- Some review effects are synchronous because the existing Git and filesystem
  adapters are synchronous. Introducing an asynchronous command runtime is a
  separate design change.
