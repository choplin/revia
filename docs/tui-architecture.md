# TUI Architecture

Revia uses a Multilayer Elm Architecture above `urushi-tui-app`. One persistent
Root model owns Global state and every mode-local model. The active mode selects
which local program receives input; it does not create or destroy that mode's
state.

## Persistent slices

| Slice | Owns |
| --- | --- |
| `app::global` | Thread state, status, pending-operation identity, running state, keyboard protocol, global bindings, and shell header/footer semantics. |
| `mode::review` | Diff request and session, focus, viewport, layout preferences, search, filters, projected threads, and review commands. |
| `mode::composer` | Draft text, grapheme cursor, reply target, scroll, validation, and explicit entry/exit reset. |
| `mode::help` | Invocation context, viewport, and help scrolling. |
| `mode::rollup` | Persistent rollup selection and thread-opening behavior. |
| `app::Model` | All slices plus the single `ActiveMode` registry. |

Each mode defines its own Model, Event, bindings, Update, view, Effect, Outcome,
and Intent vocabulary as needed. A mode update receives only its model plus a
projected input structure; it does not import Root or sibling-mode state.

## Input resolution

`tui_app` normalizes Urushi key events into framework-neutral `PhysicalInput`.
The active mode then returns one `BindingResolution`:

- `Handle(event)` handles a normal local command;
- `Override(event)` handles a local or modal command that takes precedence over
  the same physical key globally;
- `Delegate` explicitly offers the input to Global bindings;
- `Consume` stops propagation without an event;
- `Unbound` records that the active program has no meaning for the input.

Global bindings therefore run only after explicit delegation. Modal programs
consume unrelated inputs, while review delegates global quit and help commands.
Key release and unsafe repeats are consumed at the binding boundary, before
they can duplicate modal transitions or external operations.

## Update, intents, and effects

```text
PhysicalInput
    |
    v
active Mode bindings --> Mode Event --> Mode update
                                      /           \
                               local Model      Intent / Effect
                                                   |       |
                                                   v       v
                                                Root    app::Effect
                                                  |         |
                                      cross-slice change    v
                                                       Runtime I/O
                                                            |
                                                            v
                                                     owner-tagged Outcome
                                                            |
                                                            +--> Mode update
```

Mode update functions are pure state transitions. Intents request coordination
that only Root can perform, such as opening the composer, replacing Global
thread state, or selecting a thread in review. Mode effects describe external
work in the mode's vocabulary; Root lifts them into one operation-oriented
`app::Effect` vocabulary.

`Runtime` is the only interpreter for Git, filesystem, anchor, thread-store, and
wall-clock operations. Each effect receives an operation ID and owning mode.
Global records one pending operation, and accepts a completion only when ID,
owner, and operation kind all match. The typed outcome is then lowered back into
the owner's local `Outcome` and re-enters that mode's update function.

This rule prevents I/O errors from bypassing normal state transitions and keeps
late or duplicate results from clearing newer pending state. Its rationale and
extension rules are in
[`design/multilayer-elm.md`](design/multilayer-elm.md).

## Semantic and physical view

Root asks every relevant slice for a semantic projection and composes one
`semantic::View`. That value names roles and content: comparison header, file
rail, review or rollup body, footer, overlay, and layout policy. It contains no
Urushi `View` or terminal cell values.

`Renderer` converts diff and thread semantics into styled, width-bounded text.
`UrushiRenderer` then maps the semantic screen into Urushi layout primitives,
panels, viewports, scrollbars, overlays, and cursor placement. `tui_app` returns
the resulting `urushi::View` to the framework.

This separation lets scenario tests drive semantic input, events, effects,
outcomes, viewport changes, and virtual time without a terminal. Renderer tests
then verify physical resolution independently, and the PTY smoke test verifies
the real session lifecycle.

## Extension rules

- Add a mode only when it owns persistent local interaction state. Update the
  explicit `ActiveMode` registry, Root model field, event dispatch, binding
  dispatch, intent/effect lifting, and view composition together.
- Keep a cross-slice transition in Root. Do not pass Root into a local update to
  avoid writing the projection.
- Add an external operation by extending the root Effect/Outcome vocabulary and
  `Runtime`; return its completion through update rather than mutating state in
  the adapter.
- Add physical presentation in `UrushiRenderer` or the diff presentation layer,
  not in semantic update code.
- Test interaction behavior in `app::scenario`; test Git/filesystem adapters at
  their boundary and terminal lifecycle through the PTY test.

