# Multilayer Elm Application Boundary

## Rule

The application has one persistent Root model containing Global plus all
mode-local models. `ActiveMode` is the only closed mode registry. Every mode is
a complete local program with its own state, event, bindings, update, semantic
view, intents, effects, and outcomes as needed.

Mode update and view functions receive projected inputs rather than Root. They
cannot import sibling modes. Root alone:

- selects the active program;
- delegates explicitly offered input to Global;
- interprets mode intents and coordinates slices;
- lifts local effects into the operation-oriented Root effect vocabulary;
- routes owner-tagged outcomes back into the correct local update; and
- composes the complete semantic view.

External work is always `update -> Effect -> Runtime -> Outcome -> update`.
`Runtime` may use Git, filesystem, thread persistence, anchors, and the clock;
update code may not. One pending operation is tracked by ID, owner, and kind,
and any non-matching result is stale.

## Why

Persistent slices make mode retention and reset explicit. A composer draft or
rollup selection cannot vanish merely because an enum variant was replaced, and
entry transitions can deliberately reset only the state that should reset.

Projected inputs keep local programs testable and prevent Root from becoming an
implicit dependency. Explicit input resolution distinguishes modal consumption,
local override, and global delegation rather than relying on match ordering.
Returning effect results through update keeps success, failure, pending state,
and user feedback in one deterministic transition system.

The Root effect vocabulary is organized by external operation rather than by
mode. Ownership is metadata on an operation/result, so adding a mode does not
duplicate every transport variant.

## Rejected alternatives

- A single global message enum and update function makes every interaction
  change touch Root and obscures local ownership.
- Constructing only the active mode loses inactive state implicitly and makes
  persistence policy depend on enum lifetime.
- Letting modes perform I/O couples state transitions to Git and filesystem
  availability and bypasses deterministic scenario tests.
- Letting effect callbacks mutate models directly creates a second update path
  with different error and pending-state semantics.
- Using one effect/outcome enum per mode duplicates the closed mode registry and
  complicates cross-mode routing.
- Passing Root into mode update/view functions removes the boundary while
  preserving only its naming ceremony.

## Extension checklist

When adding a mode, update `ActiveMode`, Root model construction, Event dispatch,
input dispatch, intent/effect interpretation, and semantic view composition.
When adding external work, define one root operation and outcome, interpret it
in `Runtime`, and route it back to the declaring mode. Add scenario coverage for
binding precedence, pending identity, successful and failed outcomes, and mode
state retention/reset.

