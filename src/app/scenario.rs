use crate::{
    anchor::{Anchor, HunkLocation},
    app::{
        ActiveMode, Effect, EffectResult, Model, Outcome,
        global::{self, PendingEffect},
    },
    diff::{DiffDocument, DiffRequest, DiffTarget, LoadedDiff},
    input::{BindingResolution, Key, KeyPhase, PhysicalInput},
    mode::{composer, help, review, rollup},
    semantic::{Body, Overlay},
    thread::{Participant, ParticipantKind, ThreadState},
};

struct Scenario {
    model: Model,
    trace: Vec<Trace>,
    virtual_time_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Trace {
    Event(String),
    Effect(Effect),
}

impl Scenario {
    fn given(raw_diff: &str, threads: ThreadState) -> Self {
        let request = DiffRequest {
            target: DiffTarget::WorkingTree,
            context_lines: 3,
        };
        Self {
            model: Model::new(
                request,
                LoadedDiff {
                    text: raw_diff.into(),
                    document: DiffDocument::parse(raw_diff),
                },
                threads,
            ),
            trace: Vec::new(),
            virtual_time_ms: 0,
        }
    }

    fn when_event(&mut self, event: impl super::Event) -> Vec<Effect> {
        self.trace.push(Trace::Event(format!("{event:?}")));
        let effects = super::update(&mut self.model, event);
        self.trace
            .extend(effects.iter().cloned().map(Trace::Effect));
        effects
    }

    fn when_input(&mut self, input: PhysicalInput) -> BindingResolution<()> {
        let (resolution, effects) = super::handle_input(&mut self.model, input);
        self.trace
            .extend(effects.iter().cloned().map(Trace::Effect));
        resolution
    }

    fn inject(&mut self, owner: ActiveMode, outcome: Outcome) {
        self.when_event(EffectResult { owner, outcome });
    }

    fn advance_virtual_time(&mut self, milliseconds: u64) {
        self.virtual_time_ms += milliseconds;
    }
}

fn input(key: Key) -> PhysicalInput {
    PhysicalInput {
        key,
        shift: false,
        phase: KeyPhase::Press,
    }
}

fn threads() -> ThreadState {
    let mut threads = ThreadState::default();
    let human = Participant {
        id: "human".into(),
        kind: ParticipantKind::Human,
    };
    threads.post(
        Anchor::new("deadbeef", HunkLocation::new("a.rs", "@@ -1 +1 @@")),
        human.clone(),
        "one".into(),
        1,
    );
    threads.post(
        Anchor::new("deadbeef", HunkLocation::new("a.rs", "@@ -1 +1 @@")),
        human,
        "two".into(),
        2,
    );
    threads
}

const RAW: &str = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+new\n";

#[test]
fn all_mode_state_is_persistent_and_transitions_reset_explicitly() {
    let mut scenario = Scenario::given(RAW, threads());
    scenario.when_event(review::Event::BeginThread { always_new: true });
    assert_eq!(scenario.model.active_mode, ActiveMode::Composer);

    assert!(matches!(
        scenario.when_input(input(Key::Char('q'))),
        BindingResolution::Override(())
    ));
    assert_eq!(scenario.model.composer.input(), "q");
    scenario.when_event(composer::Event::Cancel);
    assert_eq!(scenario.model.active_mode, ActiveMode::Review);
    assert_eq!(scenario.model.composer.input(), "");

    scenario.when_event(review::Event::ShowRollup);
    scenario.when_event(rollup::Event::Move(1));
    scenario.when_event(rollup::Event::Close);
    scenario.when_event(review::Event::ShowRollup);
    assert_eq!(scenario.model.rollup.selected(), 1);
}

#[test]
fn active_mode_dispatches_the_mode_program() {
    let mut scenario = Scenario::given(RAW, ThreadState::default());

    scenario.when_event(composer::Event::InsertCharacter('x'));
    assert_eq!(scenario.model.composer.input(), "");

    scenario.when_event(review::Event::BeginThread { always_new: true });
    scenario.when_event(composer::Event::InsertCharacter('x'));
    assert_eq!(scenario.model.composer.input(), "x");
}

#[test]
fn binding_resolution_distinguishes_all_mode_intents() {
    assert_eq!(
        review::bindings(input(Key::Char('q'))),
        BindingResolution::Delegate
    );
    assert_eq!(
        composer::bindings(input(Key::Char('q'))),
        BindingResolution::Override(composer::Event::InsertCharacter('q'))
    );
    assert_eq!(
        help::bindings(input(Key::Char('q'))),
        BindingResolution::Consume
    );
    assert_eq!(
        review::bindings(input(Key::Other)),
        BindingResolution::Unbound
    );
    assert!(matches!(
        rollup::bindings(input(Key::Esc)),
        BindingResolution::Override(rollup::Event::Close)
    ));
}

#[test]
fn effect_outcome_returns_to_update_and_clears_pending_state() {
    let mut scenario = Scenario::given(RAW, ThreadState::default());
    let effects = scenario.when_event(review::Event::AdjustContext(1));
    assert!(matches!(
        effects.as_slice(),
        [Effect::ReloadDiff {
            owner: ActiveMode::Review,
            purpose: review::ReloadPurpose::ContextChanged,
            ..
        }]
    ));
    assert_eq!(
        scenario.model.global.pending,
        Some(PendingEffect::ReloadDiff)
    );

    let next = LoadedDiff {
        text: String::new(),
        document: DiffDocument::default(),
    };
    scenario.inject(
        ActiveMode::Review,
        Outcome::DiffReloaded {
            purpose: review::ReloadPurpose::ContextChanged,
            result: Ok(next),
        },
    );
    assert_eq!(scenario.model.global.pending, None);
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("context: 4 lines")
    );
    assert!(
        scenario
            .model
            .review
            .session()
            .diff()
            .document
            .files
            .is_empty()
    );
}

#[test]
fn failed_outcome_clears_pending_state_without_replacing_session() {
    let mut scenario = Scenario::given(RAW, ThreadState::default());
    let original_diff = scenario.model.review.session().diff().text.clone();
    scenario.when_event(review::Event::AdjustContext(1));

    scenario.inject(
        ActiveMode::Review,
        Outcome::DiffReloaded {
            purpose: review::ReloadPurpose::ContextChanged,
            result: Err("boom".into()),
        },
    );

    assert_eq!(scenario.model.global.pending, None);
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("could not reload diff: boom")
    );
    assert_eq!(scenario.model.review.session().diff().text, original_diff);
}

#[test]
fn viewport_is_scenario_input_and_controls_page_scrolling() {
    let mut scenario = Scenario::given(RAW, ThreadState::default());
    scenario.when_event(global::Event::ViewportResized(7));
    scenario.when_event(review::Event::ScrollViewport(1));

    assert_eq!(scenario.model.review.scroll(), 7);
}

#[test]
fn global_binding_is_reached_only_by_explicit_delegation() {
    let mut scenario = Scenario::given(RAW, ThreadState::default());
    assert!(matches!(
        scenario.when_input(input(Key::Char('?'))),
        BindingResolution::Handle(())
    ));
    assert_eq!(scenario.model.active_mode, ActiveMode::Help);
    assert_eq!(
        scenario.when_input(input(Key::Char('q'))),
        BindingResolution::Consume
    );
}

#[test]
fn semantic_view_preserves_roles_independently_of_ratatui_layout() {
    let mut scenario = Scenario::given(RAW, ThreadState::default());
    let view = super::view(&scenario.model);
    assert_eq!(view.header.file_count, 1);
    assert!(view.file_rail.is_some());
    assert!(matches!(view.body, Body::Review(_)));
    assert!(view.overlay.is_none());

    scenario.when_event(review::Event::BeginThread { always_new: true });
    let view = super::view(&scenario.model);
    assert!(matches!(
        view.overlay,
        Some(Overlay::Composer {
            replying: false,
            ..
        })
    ));
}

#[test]
fn scenario_trace_uses_virtual_time_without_wall_clock_dependency() {
    let mut scenario = Scenario::given(RAW, ThreadState::default());
    scenario.advance_virtual_time(250);
    scenario.when_event(review::Event::ScrollRows(1));
    assert_eq!(scenario.virtual_time_ms, 250);
    assert_eq!(scenario.model.review.scroll(), 1);
    assert_eq!(scenario.trace.len(), 1);
}
