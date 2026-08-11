use crate::{
    anchor::{Anchor, HunkLocation},
    app::{
        ActiveMode, Effect, EffectResult, Model, Outcome,
        effect::{OperationId, PendingEffectKind},
        global,
    },
    diff::{DiffDocument, DiffRequest, DiffTarget, LoadedDiff},
    input::{BindingResolution, Key, KeyPhase, PhysicalInput},
    mode::{composer, help, review, rollup},
    semantic::{Body, Overlay},
    thread::{Participant, ParticipantKind, Resolution, ThreadChange, ThreadState, ThreadSuccess},
    ui::{FocusArea, LayoutMode},
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
        let operation_id = self
            .model
            .global
            .pending
            .expect("scenario outcome requires a pending operation")
            .operation_id;
        self.inject_with_id(operation_id, owner, outcome);
    }

    fn inject_with_id(&mut self, operation_id: OperationId, owner: ActiveMode, outcome: Outcome) {
        self.when_event(EffectResult {
            operation_id,
            owner,
            outcome,
        });
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

fn repeated(key: Key) -> PhysicalInput {
    PhysicalInput {
        key,
        shift: false,
        phase: KeyPhase::Repeat,
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
const TWO_FILES: &str = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+new\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1 +1 @@\n-old_b\n+new_b\n";

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
        scenario.model.global.pending.map(|pending| pending.kind),
        Some(PendingEffectKind::ReloadDiff)
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
    scenario.when_event(global::Event::ViewportResized {
        rows: 7,
        columns: 80,
    });
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

#[test]
fn escape_unwinds_each_transient_mode_before_review_can_quit() {
    let mut scenario = Scenario::given(RAW, threads());

    scenario.when_input(input(Key::Char('c')));
    assert_eq!(scenario.model.active_mode, ActiveMode::Composer);
    scenario.when_input(input(Key::Esc));
    assert_eq!(scenario.model.active_mode, ActiveMode::Review);
    assert!(scenario.model.is_running());
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("cancelled thread draft")
    );

    scenario.when_input(input(Key::Char('?')));
    assert_eq!(scenario.model.active_mode, ActiveMode::Help);
    scenario.when_input(input(Key::Esc));
    assert_eq!(scenario.model.active_mode, ActiveMode::Review);
    assert!(scenario.model.is_running());
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("closed keyboard help")
    );

    scenario.when_input(input(Key::Char('v')));
    assert_eq!(scenario.model.active_mode, ActiveMode::Rollup);
    scenario.when_input(input(Key::Esc));
    assert_eq!(scenario.model.active_mode, ActiveMode::Review);
    assert!(scenario.model.is_running());
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("closed thread rollup")
    );

    scenario.when_input(input(Key::Esc));
    assert!(!scenario.model.is_running());
}

#[test]
fn q_only_quits_from_review() {
    let mut scenario = Scenario::given(RAW, threads());

    scenario.when_input(input(Key::Char('c')));
    assert!(matches!(
        scenario.when_input(input(Key::Char('q'))),
        BindingResolution::Override(())
    ));
    assert_eq!(scenario.model.composer.input(), "q");
    assert!(scenario.model.is_running());
    scenario.when_input(input(Key::Esc));

    scenario.when_input(input(Key::Char('?')));
    assert_eq!(
        scenario.when_input(input(Key::Char('q'))),
        BindingResolution::Consume
    );
    assert_eq!(scenario.model.active_mode, ActiveMode::Help);
    assert!(scenario.model.is_running());
    scenario.when_input(input(Key::Esc));

    scenario.when_input(input(Key::Char('v')));
    assert_eq!(
        scenario.when_input(input(Key::Char('q'))),
        BindingResolution::Consume
    );
    assert_eq!(scenario.model.active_mode, ActiveMode::Rollup);
    assert!(scenario.model.is_running());
    scenario.when_input(input(Key::Esc));

    scenario.when_input(input(Key::Char('q')));
    assert!(!scenario.model.is_running());
}

#[test]
fn one_review_cursor_drives_file_hunk_thread_and_stream_targets() {
    let mut scenario = Scenario::given(TWO_FILES, threads());

    scenario.when_input(input(Key::Char('.')));
    let view = super::view(&scenario.model);
    assert_eq!(
        view.file_rail.as_ref().and_then(|rail| rail.selected),
        Some(1)
    );
    assert_eq!(scenario.model.review.focus(), FocusArea::Review);
    assert!(
        scenario
            .model
            .global
            .status
            .as_deref()
            .is_some_and(|status| status.contains("file 2/2: b.rs"))
    );

    scenario.when_input(input(Key::Char(',')));
    scenario.when_input(input(Key::Char('t')));
    assert_eq!(scenario.model.review.focus(), FocusArea::Threads);
    let view = super::view(&scenario.model);
    assert!(view.footer.current_context.text.contains("thread #0"));
    assert!(view.footer.contextual_keys.text.contains("x resolve"));

    scenario.when_input(input(Key::Char('t')));
    assert!(
        scenario
            .model
            .global
            .status
            .as_deref()
            .is_some_and(|status| status.contains("thread target: #1 (2/2)"))
    );
    scenario.when_input(input(Key::Char('T')));
    assert!(
        scenario
            .model
            .global
            .status
            .as_deref()
            .is_some_and(|status| status.contains("thread target: #0 (1/2)"))
    );

    scenario.when_input(input(Key::Tab));
    assert_eq!(scenario.model.review.focus(), FocusArea::Review);
    let previous_scroll = scenario.model.review.scroll();
    scenario.when_input(input(Key::Char('j')));
    assert_eq!(scenario.model.review.scroll(), previous_scroll + 1);
    assert!(
        scenario
            .model
            .global
            .status
            .as_deref()
            .is_some_and(|status| status == format!("review stream row {}", previous_scroll + 2))
    );
}

#[test]
fn contextual_keys_follow_mode_and_visible_thread_availability() {
    let mut empty = Scenario::given(RAW, ThreadState::default());
    let view = super::view(&empty.model);
    assert!(view.footer.contextual_keys.text.contains("c new thread"));
    assert!(!view.footer.contextual_keys.text.contains("x resolve"));
    empty.when_input(input(Key::Tab));
    assert_eq!(empty.model.review.focus(), FocusArea::Review);
    assert_eq!(
        empty.model.global.status.as_deref(),
        Some("cannot focus threads: this hunk has no threads")
    );

    empty.when_input(input(Key::Char('c')));
    let view = super::view(&empty.model);
    assert_eq!(view.footer.contextual_keys.text, "Enter post • Esc cancel");
    empty.when_input(input(Key::Esc));
    empty.when_input(input(Key::Char('?')));
    let view = super::view(&empty.model);
    assert_eq!(view.footer.contextual_keys.text, "Esc/? close help");
    empty.when_input(input(Key::Esc));
    empty.when_input(input(Key::Char('v')));
    let view = super::view(&empty.model);
    assert!(
        view.footer
            .contextual_keys
            .text
            .contains("no thread targets")
    );
    assert!(
        view.footer
            .current_context
            .text
            .contains("Target: no threads")
    );
}

#[test]
fn handled_view_and_unavailable_actions_report_results() {
    let mut scenario = Scenario::given(RAW, threads());

    scenario.when_input(input(Key::Char('1')));
    assert_eq!(scenario.model.review.focus(), FocusArea::Review);
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("layout: split")
    );
    scenario.when_input(input(Key::Char('s')));
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("file rail hidden")
    );
    scenario.when_input(input(Key::Char('.')));
    assert!(super::view(&scenario.model).file_rail.is_none());
    assert!(
        scenario
            .model
            .global
            .status
            .as_deref()
            .is_some_and(|status| status.contains("file 1/1: a.rs"))
    );
    scenario.when_input(input(Key::Char('m')));
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("hunk headers hidden")
    );
    scenario.when_input(input(Key::Char('w')));
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("line wrapping enabled")
    );

    let effects = scenario.when_input(input(Key::Char('x')));
    assert_eq!(effects, BindingResolution::Handle(()));
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("could not close thread: select a thread with t first")
    );
    assert_eq!(scenario.model.global.pending, None);

    scenario.when_event(review::Event::AdjustContext(-4));
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("context already has 0 lines")
    );

    let mut empty = Scenario::given("", ThreadState::default());
    empty.when_input(input(Key::Char('.')));
    assert_eq!(
        empty.model.global.status.as_deref(),
        Some("cannot move files: this diff has no changed files")
    );
    empty.when_input(input(Key::Char(']')));
    assert_eq!(
        empty.model.global.status.as_deref(),
        Some("cannot move hunks: this diff has no hunks")
    );
    empty.when_input(input(Key::Char('v')));
    empty.when_input(input(Key::Enter));
    assert_eq!(
        empty.model.global.status.as_deref(),
        Some("cannot jump: rollup has no threads")
    );

    // Keep the explicit layout type exercised in the scenario contract.
    scenario.when_event(review::Event::SetLayout(LayoutMode::Auto));
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("layout: responsive")
    );
}

#[test]
fn pending_operation_rejects_duplicate_mutation_and_remains_visible() {
    let mut scenario = Scenario::given(RAW, ThreadState::default());
    let first = scenario.when_event(review::Event::ReloadDiff);
    assert_eq!(first.len(), 1);
    assert_eq!(
        scenario.model.global.pending.map(|pending| pending.kind),
        Some(PendingEffectKind::ReloadDiff)
    );

    let duplicate = scenario.when_event(review::Event::ReloadDiff);
    assert!(duplicate.is_empty());
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("cannot start another action while an operation is pending")
    );
    let view = super::view(&scenario.model);
    assert!(
        view.footer
            .current_context
            .text
            .contains("Pending: reloading diff…")
    );

    scenario.inject(
        ActiveMode::Review,
        Outcome::DiffReloaded {
            purpose: review::ReloadPurpose::Manual,
            result: Err("repository unavailable".into()),
        },
    );
    assert_eq!(scenario.model.global.pending, None);
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("could not reload diff: repository unavailable")
    );
}

#[test]
fn thread_mutation_reports_pending_failure_and_success_for_visible_target() {
    let mut scenario = Scenario::given(RAW, threads());
    scenario.when_input(input(Key::Char('t')));

    scenario.when_input(input(Key::Char('x')));
    assert_eq!(
        scenario.model.global.pending.map(|pending| pending.kind),
        Some(PendingEffectKind::ChangeThreads)
    );
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("updating thread…")
    );
    scenario.inject(
        ActiveMode::Review,
        Outcome::ThreadsChanged {
            result: Err("could not close thread: denied".into()),
        },
    );
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("could not close thread: denied")
    );

    scenario.when_input(input(Key::Char('x')));
    let mut changed = scenario.model.global.threads.clone();
    let id = changed.ordered_ids()[0];
    let human = Participant {
        id: "human".into(),
        kind: ParticipantKind::Human,
    };
    changed.close(id, &human).unwrap();
    scenario.inject(
        ActiveMode::Review,
        Outcome::ThreadsChanged {
            result: Ok(ThreadChange {
                state: changed,
                success: ThreadSuccess::Closed,
            }),
        },
    );
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("thread closed")
    );
    assert!(matches!(
        scenario
            .model
            .global
            .threads
            .thread(id)
            .map(|thread| &thread.resolution),
        Some(Resolution::Resolved)
    ));
}

#[test]
fn stale_effect_result_cannot_clear_or_replace_the_current_operation() {
    let mut scenario = Scenario::given(RAW, ThreadState::default());
    scenario.when_event(review::Event::ReloadDiff);
    let first_id = scenario.model.global.pending.unwrap().operation_id;
    scenario.inject(
        ActiveMode::Review,
        Outcome::DiffReloaded {
            purpose: review::ReloadPurpose::Manual,
            result: Ok(LoadedDiff {
                text: RAW.into(),
                document: DiffDocument::parse(RAW),
            }),
        },
    );

    scenario.when_event(review::Event::ReloadDiff);
    let second = scenario.model.global.pending.unwrap();
    assert_ne!(first_id, second.operation_id);
    scenario.inject_with_id(
        first_id,
        ActiveMode::Review,
        Outcome::DiffReloaded {
            purpose: review::ReloadPurpose::Manual,
            result: Ok(LoadedDiff {
                text: String::new(),
                document: DiffDocument::default(),
            }),
        },
    );

    assert_eq!(scenario.model.global.pending, Some(second));
    assert_eq!(scenario.model.review.session().diff().text, RAW);
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("ignored stale operation result")
    );
    assert!(
        super::view(&scenario.model)
            .footer
            .current_context
            .text
            .contains("Pending: reloading diff…")
    );
}

#[test]
fn reload_drops_thread_focus_when_the_canonical_target_disappears() {
    let mut state = threads();
    let human = Participant {
        id: "human".into(),
        kind: ParticipantKind::Human,
    };
    state.post(
        Anchor::new("deadbeef", HunkLocation::new("b.rs", "@@ -1 +1 @@")),
        human,
        "on b".into(),
        3,
    );
    let mut scenario = Scenario::given(TWO_FILES, state);
    scenario.when_input(input(Key::Char('t')));
    assert_eq!(scenario.model.review.focus(), FocusArea::Threads);
    scenario.when_event(review::Event::ReloadDiff);

    let only_b = "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1 +1 @@\n-old_b\n+new_b\n";
    scenario.inject(
        ActiveMode::Review,
        Outcome::DiffReloaded {
            purpose: review::ReloadPurpose::Manual,
            result: Ok(LoadedDiff {
                text: only_b.into(),
                document: DiffDocument::parse(only_b),
            }),
        },
    );

    assert_eq!(scenario.model.review.focus(), FocusArea::Review);
    assert!(
        super::view(&scenario.model)
            .footer
            .current_context
            .text
            .contains("b.rs")
    );
    scenario.when_input(input(Key::Char('x')));
    assert_eq!(scenario.model.global.pending, None);
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("could not close thread: select a thread with t first")
    );
}

#[test]
fn pending_reload_rejects_composer_entry_without_moving_the_target() {
    let mut scenario = Scenario::given(RAW, threads());
    let location = scenario.model.review.selected_location();
    scenario.when_event(review::Event::ReloadDiff);
    let pending = scenario.model.global.pending;

    scenario.when_input(input(Key::Char('c')));

    assert_eq!(scenario.model.active_mode, ActiveMode::Review);
    assert_eq!(scenario.model.review.selected_location(), location);
    assert_eq!(scenario.model.global.pending, pending);
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("cannot start another action while an operation is pending")
    );
}

#[test]
fn repeated_modal_keys_cannot_unwind_more_than_one_state() {
    let mut scenario = Scenario::given(RAW, threads());

    scenario.when_input(input(Key::Char('c')));
    scenario.when_input(input(Key::Esc));
    scenario.when_input(repeated(Key::Esc));
    assert_eq!(scenario.model.active_mode, ActiveMode::Review);
    assert!(scenario.model.is_running());

    scenario.when_input(input(Key::Char('?')));
    scenario.when_input(repeated(Key::Char('?')));
    assert_eq!(scenario.model.active_mode, ActiveMode::Help);
    scenario.when_input(input(Key::Esc));

    scenario.when_input(input(Key::Char('v')));
    scenario.when_input(repeated(Key::Char('v')));
    assert_eq!(scenario.model.active_mode, ActiveMode::Rollup);
    assert!(scenario.model.is_running());
}

#[test]
fn narrow_footer_and_rail_feedback_remain_truthful() {
    let mut scenario = Scenario::given(RAW, threads());
    scenario.when_event(global::Event::ViewportResized {
        rows: 16,
        columns: 64,
    });
    scenario.when_input(input(Key::Char('s')));
    scenario.when_input(input(Key::Char('s')));
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("file rail enabled; hidden below 72 columns")
    );
    let view = super::view(&scenario.model);
    assert!(view.footer.current_context.text.contains("a.rs"));
    assert!(view.footer.current_context.text.contains("rail hidden"));

    scenario.when_input(input(Key::Char('t')));
    let view = super::view(&scenario.model);
    assert!(view.footer.current_context.text.contains("thread #0"));
    assert!(view.footer.contextual_keys.text.starts_with("Tab stream"));
    assert!(view.footer.contextual_keys.text.contains("x/R"));
}
