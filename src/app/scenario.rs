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
    presentation::{self, ReviewRowMap},
    semantic::{Body, DiffSearchTarget, LayoutPolicy, Overlay, ReviewBody},
    thread::{
        Participant, ParticipantKind, Resolution, ThreadChange, ThreadId, ThreadState,
        ThreadSuccess,
    },
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

fn type_search(scenario: &mut Scenario, query: &str) {
    scenario.when_input(input(Key::Char('/')));
    for character in query.chars() {
        scenario.when_input(input(Key::Char(character)));
    }
}

fn review_geometry(model: &Model) -> (ReviewBody, LayoutPolicy, ReviewRowMap) {
    let view = super::view(model);
    let Body::Review(body) = view.body else {
        panic!("scenario is not displaying the review body");
    };
    let rows = presentation::review_row_map(&body, body.viewport.presentation_width, view.layout);
    (body, view.layout, rows)
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

fn lifecycle_threads() -> ThreadState {
    let mut threads = ThreadState::default();
    let human = Participant {
        id: "reviewer".into(),
        kind: ParticipantKind::Human,
    };
    threads.post(
        Anchor::new("deadbeef", HunkLocation::new("a.rs", "@@ -1 +1 @@")),
        human.clone(),
        "open context".into(),
        1,
    );
    let resolved = threads.post(
        Anchor::new("deadbeef", HunkLocation::new("a.rs", "@@ -1 +1 @@")),
        human.clone(),
        "resolved provenance".into(),
        2,
    );
    threads.close(resolved, &human).unwrap();
    let attention = threads.post(
        Anchor::new("deadbeef", HunkLocation::new("a.rs", "@@ -1 +1 @@")),
        human,
        "wide 画面 context requiring an explicit decision".into(),
        3,
    );
    threads.set_needs_attention(attention, true).unwrap();
    threads.set_outdated(attention, true).unwrap();
    threads
}

fn rail_selection(scenario: &Scenario) -> Option<usize> {
    super::view(&scenario.model)
        .file_rail
        .as_ref()
        .and_then(|rail| rail.selected)
}

#[test]
fn the_file_rail_scopes_vertical_keys_and_leaves_global_keys_alone() {
    let mut scenario = Scenario::given(TWO_FILES, ThreadState::default());
    assert_eq!(scenario.model.review.focus(), FocusArea::Review);

    // This diff carries no threads, so Tab steps over the thread region.
    scenario.when_input(input(Key::Tab));
    assert_eq!(scenario.model.review.focus(), FocusArea::Files);

    // j/k address whole files here rather than scrolling the diff.
    assert_eq!(rail_selection(&scenario), Some(0));
    scenario.when_input(input(Key::Char('j')));
    assert_eq!(rail_selection(&scenario), Some(1));
    scenario.when_input(input(Key::Char('k')));
    assert_eq!(rail_selection(&scenario), Some(0));

    // Keys the scoped layer does not claim keep their global meaning, and
    // hiding the rail cannot leave focus stranded on it.
    scenario.when_input(input(Key::Char('s')));
    assert_eq!(scenario.model.review.focus(), FocusArea::Review);
    assert!(super::view(&scenario.model).file_rail.is_none());
}

#[test]
fn shared_navigation_acts_from_the_rail_without_taking_its_focus() {
    let mut scenario = Scenario::given(TWO_FILES, ThreadState::default());
    scenario.when_input(input(Key::Tab));
    assert_eq!(scenario.model.review.focus(), FocusArea::Files);

    // Navigation is shared across regions: it does the work and leaves focus
    // where the user put it.
    for key in [
        Key::Char('d'),
        Key::Char('u'),
        Key::Char('f'),
        Key::Char('b'),
        Key::Char('g'),
        Key::Char('G'),
        Key::Char(']'),
        Key::Char('['),
    ] {
        scenario.when_input(input(key));
        assert_eq!(
            scenario.model.review.focus(),
            FocusArea::Files,
            "{key:?} moved focus away from the rail"
        );
    }

    let before = rail_selection(&scenario);
    scenario.when_input(input(Key::Char('.')));
    assert_ne!(rail_selection(&scenario), before);
    assert_eq!(scenario.model.review.focus(), FocusArea::Files);
    scenario.when_input(input(Key::Char(',')));
    assert_eq!(rail_selection(&scenario), before);
    assert_eq!(scenario.model.review.focus(), FocusArea::Files);
}

#[test]
fn scrolling_keeps_a_selected_inline_thread_focused() {
    let mut scenario = Scenario::given(RAW, threads());
    scenario.when_input(input(Key::Char('t')));
    assert_eq!(scenario.model.review.focus(), FocusArea::Threads);

    scenario.when_input(input(Key::Char('d')));
    assert_eq!(scenario.model.review.focus(), FocusArea::Threads);
    scenario.when_input(input(Key::Char('u')));
    assert_eq!(scenario.model.review.focus(), FocusArea::Threads);

    // Tab is the way out, and it returns to the diff rather than the rail.
    scenario.when_input(input(Key::Tab));
    assert_eq!(scenario.model.review.focus(), FocusArea::Review);
}

#[test]
fn a_shell_too_narrow_to_draw_the_rail_keeps_focus_off_it() {
    let mut scenario = Scenario::given(TWO_FILES, ThreadState::default());
    scenario.when_input(input(Key::Tab));
    assert_eq!(scenario.model.review.focus(), FocusArea::Files);

    scenario.when_event(global::Event::ViewportResized {
        rows: 20,
        columns: 48,
    });
    assert_eq!(scenario.model.review.focus(), FocusArea::Review);

    scenario.when_input(input(Key::Tab));
    assert_eq!(scenario.model.review.focus(), FocusArea::Review);
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("cannot focus the file rail: it is hidden; press s to show it")
    );
}

const RAW: &str = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+new\n";
const TWO_FILES: &str = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+new\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1 +1 @@\n-old_b\n+new_b\n";
const FILTER_DIFF: &str = concat!(
    "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n",
    "@@ -1 +1 @@ first\n-old_a1\n+new_a1\n",
    "@@ -10 +10 @@ second\n-old_a10\n+new_a10\n",
    "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n",
    "@@ -1 +1 @@ only\n-old_b\n+new_b\n",
    "diff --git a/c.rs b/c.rs\n--- a/c.rs\n+++ b/c.rs\n",
    "@@ -1 +1 @@ only\n-old_c\n+new_c\n",
);
const CONTEXT_U3_DIFF: &str = concat!(
    "diff --git a/context.rs b/context.rs\n--- a/context.rs\n+++ b/context.rs\n",
    "@@ -30,6 +30,49 @@ fn context()\n-old\n+new\n",
);
const CONTEXT_U4_DIFF: &str = concat!(
    "diff --git a/context.rs b/context.rs\n--- a/context.rs\n+++ b/context.rs\n",
    "@@ -29,8 +29,51 @@ fn context()\n before\n-old\n+new\n after\n",
);
const MERGED_ANCHOR_DIFF: &str = concat!(
    "diff --git a/split.rs b/split.rs\n--- a/split.rs\n+++ b/split.rs\n",
    "@@ -20,20 +20,20 @@ merged\n-old\n+new\n",
);
const SPLIT_HUNKS_DIFF: &str = concat!(
    "diff --git a/split.rs b/split.rs\n--- a/split.rs\n+++ b/split.rs\n",
    "@@ -10,15 +10,15 @@ first\n c10\n c11\n c12\n c13\n c14\n c15\n c16\n c17\n c18\n c19\n c20\n c21\n c22\n c23\n-old_first\n+new_first\n",
    "@@ -30,15 +30,15 @@ second\n c30\n c31\n c32\n c33\n c34\n c35\n-old_second\n+new_second\n",
);

fn filter_threads() -> (ThreadState, [ThreadId; 5]) {
    let mut state = ThreadState::default();
    let human = Participant {
        id: "reviewer".into(),
        kind: ParticipantKind::Human,
    };
    let b_attention = state.post(
        Anchor::new("deadbeef", HunkLocation::new("b.rs", "@@ -1 +1 @@ only")),
        human.clone(),
        "resolved attention in b".into(),
        1,
    );
    state.close(b_attention, &human).unwrap();
    state.set_needs_attention(b_attention, true).unwrap();
    let c_attention = state.post(
        Anchor::new("deadbeef", HunkLocation::new("c.rs", "@@ -1 +1 @@ only")),
        human.clone(),
        "open attention in c".into(),
        2,
    );
    state.set_needs_attention(c_attention, true).unwrap();
    let a_attention = state.post(
        Anchor::new("deadbeef", HunkLocation::new("a.rs", "@@ -1 +1 @@ first")),
        human.clone(),
        "open attention in a".into(),
        3,
    );
    state.set_needs_attention(a_attention, true).unwrap();
    let a_open = state.post(
        Anchor::new(
            "deadbeef",
            HunkLocation::new("a.rs", "@@ -10 +10 @@ second"),
        ),
        human.clone(),
        "ordinary open in a".into(),
        4,
    );
    let a_resolved = state.post(
        Anchor::new("deadbeef", HunkLocation::new("a.rs", "@@ -1 +1 @@ first")),
        human.clone(),
        "ordinary resolved in a".into(),
        5,
    );
    state.close(a_resolved, &human).unwrap();
    (
        state,
        [a_attention, b_attention, c_attention, a_open, a_resolved],
    )
}
const LONG_DIFF: &str = concat!(
    "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n",
    "@@ -1,5 +1,5 @@ first\n one\n two\n-old three\n+new three\n four\n five\n",
    "@@ -20,4 +20,4 @@ second\n twenty\n-old twenty one\n+new twenty one\n twenty two\n twenty three\n",
    "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n",
    "@@ -40,6 +40,6 @@ middle\n forty\n forty one\n-old forty two\n+new forty two with a line that wraps across a narrow review viewport for geometry\n forty three\n forty four\n forty five\n",
    "diff --git a/c.rs b/c.rs\n--- a/c.rs\n+++ b/c.rs\n",
    "@@ -80,5 +80,5 @@ last\n eighty\n eighty one\n-old eighty two\n+new eighty two\n eighty three\n eighty four\n",
);
const RELOADED_LONG_DIFF: &str = concat!(
    "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n",
    "@@ -42,4 +42,4 @@ middle\n forty one\n-old forty two\n+new forty two\n forty three\n forty four\n",
    "diff --git a/c.rs b/c.rs\n--- a/c.rs\n+++ b/c.rs\n",
    "@@ -80,3 +80,3 @@ last\n eighty\n-old eighty one\n+new eighty one\n eighty two\n",
);

const SEARCH_DIFF: &str = concat!(
    "diff --git a/alpha.rs b/alpha.rs\n--- a/alpha.rs\n+++ b/alpha.rs\n",
    "@@ -1 +1 @@ first\n-old first\n+Needle first\n",
    "@@ -20 +20 @@ second\n-old second\n+needle second\n",
    "diff --git a/needle.rs b/needle.rs\n--- a/needle.rs\n+++ b/needle.rs\n",
    "@@ -1 +1 @@ last\n-old last\n+final NEEDLE\n",
);

#[test]
fn incremental_search_uses_semantic_git_order_and_wraps_both_directions() {
    let mut scenario = Scenario::given(SEARCH_DIFF, ThreadState::default());

    type_search(&mut scenario, "NeEdLe");
    assert_eq!(
        scenario.model.review.selected_location(),
        Some(HunkLocation::new("alpha.rs", "@@ -1 +1 @@ first"))
    );
    assert_eq!(
        scenario.model.review.search_summary(),
        Some(review::SearchSummary {
            query: "NeEdLe".into(),
            selected: Some(0),
            match_count: 4,
            editing: true,
        })
    );

    scenario.when_input(input(Key::Enter));
    scenario.when_input(input(Key::Char('n')));
    assert_eq!(
        scenario.model.review.selected_location(),
        Some(HunkLocation::new("alpha.rs", "@@ -20 +20 @@ second"))
    );
    scenario.when_input(input(Key::Char('n')));
    assert_eq!(
        scenario.model.review.selected_location(),
        Some(HunkLocation::new("needle.rs", "@@ -1 +1 @@ last"))
    );
    scenario.when_input(input(Key::Char('n')));
    scenario.when_input(input(Key::Char('n')));
    assert_eq!(
        scenario.model.review.selected_location(),
        Some(HunkLocation::new("alpha.rs", "@@ -1 +1 @@ first"))
    );
    assert!(
        scenario
            .model
            .global
            .status
            .as_deref()
            .is_some_and(|status| status.contains("(wrapped)"))
    );
    scenario.when_input(input(Key::Char('N')));
    assert_eq!(
        scenario.model.review.selected_location(),
        Some(HunkLocation::new("needle.rs", "@@ -1 +1 @@ last"))
    );
}

#[test]
fn search_context_always_reports_current_and_total_matches() {
    let mut scenario = Scenario::given(SEARCH_DIFF, ThreadState::default());
    type_search(&mut scenario, "needle");

    let editing = super::view(&scenario.model);
    assert!(editing.footer.current_context.text.contains("match 1/4"));

    scenario.when_input(input(Key::Enter));
    scenario.when_input(input(Key::Char('n')));
    let results = super::view(&scenario.model);
    assert!(results.footer.current_context.text.contains("match 2/4"));
}

#[test]
fn search_jumps_keep_the_target_away_from_viewport_edges() {
    let mut scenario = Scenario::given(LONG_DIFF, ThreadState::default());
    scenario.when_event(global::Event::ViewportResized {
        rows: 8,
        columns: 80,
    });
    type_search(&mut scenario, "middle");

    let (body, _, rows) = review_geometry(&scenario.model);
    let target = rows
        .row_for_search_target(&DiffSearchTarget::HunkHeader {
            location: HunkLocation::new("b.rs", "@@ -40,6 +40,6 @@ middle"),
        })
        .expect("search target has a physical row");
    assert_eq!(
        target,
        body.scroll
            + body
                .viewport
                .visible_rows
                .saturating_sub(1)
                .saturating_sub(2)
    );
}

#[test]
fn deleted_file_search_targets_keep_unique_paths() {
    let raw = concat!(
        "diff --git a/old-a.rs b/old-a.rs\ndeleted file mode 100644\n",
        "--- a/old-a.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-unique_a\n",
        "diff --git a/old-b.rs b/old-b.rs\ndeleted file mode 100644\n",
        "--- a/old-b.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-unique_b\n",
    );
    let mut scenario = Scenario::given(raw, ThreadState::default());
    type_search(&mut scenario, "unique_b");
    assert_eq!(
        scenario.model.review.selected_location(),
        Some(HunkLocation::new("old-b.rs", "@@ -1 +0,0 @@"))
    );

    scenario.when_input(input(Key::Esc));
    type_search(&mut scenario, "old-b.rs");
    assert_eq!(
        scenario.model.review.search_summary().unwrap().match_count,
        1
    );
    assert_eq!(
        scenario.model.review.selected_location(),
        Some(HunkLocation::new("old-b.rs", "@@ -1 +0,0 @@"))
    );
}

#[test]
fn empty_and_missing_search_queries_are_visible_and_do_not_move() {
    let mut scenario = Scenario::given(SEARCH_DIFF, ThreadState::default());
    scenario.when_event(review::Event::MoveHunk(1));
    let location = scenario.model.review.selected_location();

    scenario.when_input(input(Key::Char('/')));
    let view = super::view(&scenario.model);
    assert!(view.footer.current_context.text.contains("empty query"));
    scenario.when_input(input(Key::Enter));
    assert_eq!(scenario.model.review.selected_location(), location);
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("search query is empty; type text or press Esc to cancel")
    );

    for character in "absent".chars() {
        scenario.when_input(input(Key::Char(character)));
    }
    assert_eq!(scenario.model.review.selected_location(), location);
    assert!(
        scenario
            .model
            .global
            .status
            .as_deref()
            .is_some_and(|status| status.contains("no matches"))
    );
}

#[test]
fn cancelling_search_restores_exact_cursor_and_viewport() {
    let mut scenario = Scenario::given(LONG_DIFF, ThreadState::default());
    scenario.when_event(global::Event::ViewportResized {
        rows: 3,
        columns: 80,
    });
    scenario.when_event(review::Event::MoveHunk(1));
    scenario.when_event(review::Event::JumpToDiffEdge { end: true });
    scenario.when_event(review::Event::ScrollRows(-2));
    let location = scenario.model.review.selected_location();
    let (before, _, _) = review_geometry(&scenario.model);

    type_search(&mut scenario, "first");
    assert_ne!(scenario.model.review.selected_location(), location);
    assert!(matches!(
        scenario.when_input(input(Key::Esc)),
        BindingResolution::Override(())
    ));

    let (after, _, _) = review_geometry(&scenario.model);
    assert_eq!(scenario.model.review.selected_location(), location);
    assert_eq!(after.scroll, before.scroll);
    assert!(scenario.model.is_running());
}

#[test]
fn search_editing_handles_unicode_backspace_and_escape_repeat_without_unwinding() {
    let raw =
        "diff --git a/画面.rs b/画面.rs\n--- a/画面.rs\n+++ b/画面.rs\n@@ -1 +1 @@\n-old\n+new\n";
    let mut scenario = Scenario::given(raw, ThreadState::default());
    type_search(&mut scenario, "画a");
    assert_eq!(scenario.model.review.search_summary().unwrap().query, "画a");
    scenario.when_input(input(Key::Backspace));
    assert_eq!(scenario.model.review.search_summary().unwrap().query, "画");
    assert_eq!(
        scenario.model.review.search_summary().unwrap().match_count,
        1
    );

    assert!(matches!(
        scenario.when_input(repeated(Key::Esc)),
        BindingResolution::Consume
    ));
    assert!(scenario.model.review.search_summary().is_some());
    assert!(scenario.model.is_running());
    scenario.when_input(input(Key::Esc));
    assert!(scenario.model.review.search_summary().is_none());
    assert!(scenario.model.is_running());

    for query in ["e\u{301}", "👨‍👩‍👧‍👦"] {
        type_search(&mut scenario, query);
        scenario.when_input(input(Key::Backspace));
        assert_eq!(scenario.model.review.search_summary().unwrap().query, "");
        scenario.when_input(input(Key::Esc));
    }

    type_search(&mut scenario, "画");
    scenario.when_input(input(Key::Enter));
    assert!(matches!(
        scenario.when_input(repeated(Key::Esc)),
        BindingResolution::Consume
    ));
    assert!(scenario.model.review.search_summary().is_some());
    scenario.when_input(input(Key::Esc));
    assert!(scenario.model.review.search_summary().is_none());
    assert!(scenario.model.is_running());
    scenario.when_input(input(Key::Esc));
    assert!(!scenario.model.is_running());
}

#[test]
fn cancelling_search_restores_inline_thread_focus_and_target() {
    let mut scenario = Scenario::given(RAW, threads());
    scenario.when_input(input(Key::Char('t')));
    assert_eq!(scenario.model.review.focus(), FocusArea::Threads);
    let thread = scenario
        .model
        .review
        .selected_thread_id(&scenario.model.global.threads);

    type_search(&mut scenario, "new");
    assert_eq!(scenario.model.review.focus(), FocusArea::Review);
    scenario.when_input(input(Key::Esc));

    assert_eq!(scenario.model.review.focus(), FocusArea::Threads);
    assert_eq!(
        scenario
            .model
            .review
            .selected_thread_id(&scenario.model.global.threads),
        thread
    );
}

#[test]
fn active_search_reveals_its_semantic_match_after_resize_and_layout_changes() {
    let mut scenario = Scenario::given(LONG_DIFF, ThreadState::default());
    scenario.when_event(global::Event::ViewportResized {
        rows: 6,
        columns: 80,
    });
    type_search(&mut scenario, "geometry");

    for event in [
        review::Event::SetLayout(LayoutMode::Split),
        review::Event::SetLayout(LayoutMode::Stack),
        review::Event::ToggleWrap,
        review::Event::ToggleHunkHeaders,
    ] {
        scenario.when_event(event);
        let (body, _, rows) = review_geometry(&scenario.model);
        let target = rows
            .row_for_search_target(
                body.search_target
                    .as_ref()
                    .expect("active search has a semantic target"),
            )
            .expect("semantic search target resolves after relayout");
        assert!(target >= body.scroll);
        assert!(target < body.scroll + body.viewport.visible_rows);
    }
    scenario.when_event(global::Event::ViewportResized {
        rows: 4,
        columns: 64,
    });
    let (body, _, rows) = review_geometry(&scenario.model);
    let target = rows
        .row_for_search_target(body.search_target.as_ref().unwrap())
        .unwrap();
    assert!(target >= body.scroll);
    assert!(target < body.scroll + body.viewport.visible_rows);

    scenario.when_input(input(Key::Esc));
    let (restored, _, _) = review_geometry(&scenario.model);
    assert!(
        restored.scroll
            <= restored
                .viewport
                .total_rows
                .saturating_sub(restored.viewport.visible_rows)
    );
}

#[test]
fn successful_reload_clears_stale_search_and_failed_reload_preserves_it() {
    let mut scenario = Scenario::given(SEARCH_DIFF, ThreadState::default());
    type_search(&mut scenario, "needle");
    scenario.when_input(input(Key::Enter));
    scenario.when_event(review::Event::ReloadDiff);
    scenario.inject(
        ActiveMode::Review,
        Outcome::DiffReloaded {
            purpose: review::ReloadPurpose::Manual,
            result: Err("boom".into()),
        },
    );
    assert!(scenario.model.review.search_summary().is_some());

    scenario.when_event(review::Event::ReloadDiff);
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
    assert_eq!(scenario.model.review.search_summary(), None);
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("reloaded current diff; cleared search because the diff changed")
    );
}

#[test]
fn help_marks_invocation_commands_and_returns_to_search_location() {
    let mut scenario = Scenario::given(SEARCH_DIFF, ThreadState::default());
    scenario.when_event(review::Event::MoveHunk(1));
    let invocation = scenario.model.review.selected_location();
    type_search(&mut scenario, "needle");
    scenario.when_input(input(Key::Enter));
    let location = scenario.model.review.selected_location();
    let view = super::view(&scenario.model);
    assert!(view.footer.contextual_keys.text.contains("n/N"));
    assert!(view.footer.contextual_keys.text.contains("/ new search"));

    scenario.when_input(input(Key::Char('?')));
    let view = super::view(&scenario.model);
    let Some(Overlay::Help(help)) = view.overlay else {
        panic!("help overlay is visible");
    };
    let text = help.lines.join("\n");
    assert!(text.contains("commands valid from search results"));
    assert!(text.contains("◆ n/N next/previous match (wrap)"));
    assert!(text.contains("Navigation"));
    assert!(text.contains("View"));
    assert!(text.contains("Review actions"));

    scenario.when_input(input(Key::Esc));
    assert_eq!(scenario.model.active_mode, ActiveMode::Review);
    assert_eq!(scenario.model.review.selected_location(), location);
    assert!(scenario.model.review.search_summary().is_some());
    scenario.when_input(input(Key::Esc));
    assert_eq!(scenario.model.review.selected_location(), invocation);
    assert!(scenario.model.review.search_summary().is_none());
}

#[test]
fn help_emphasizes_thread_commands_only_for_a_thread_target() {
    let mut scenario = Scenario::given(RAW, threads());
    scenario.when_input(input(Key::Char('?')));
    let Some(Overlay::Help(help)) = super::view(&scenario.model).overlay else {
        panic!("help overlay is visible");
    };
    let text = help.lines.join("\n");
    assert!(text.contains("commands valid from diff"));
    assert!(text.contains("· x/R resolve/reopen"));
    scenario.when_input(input(Key::Esc));

    scenario.when_input(input(Key::Char('t')));
    let footer = super::view(&scenario.model).footer.contextual_keys.text;
    assert!(footer.contains("x resolve"));
    scenario.when_input(input(Key::Char('?')));
    let Some(Overlay::Help(help)) = super::view(&scenario.model).overlay else {
        panic!("help overlay is visible");
    };
    let text = help.lines.join("\n");
    assert!(text.contains("commands valid from inline thread"));
    assert!(text.contains("◆ x/R resolve/reopen"));
    assert!(text.contains("◆ a/o flags"));
}

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
    assert_eq!(scenario.model.active_mode, ActiveMode::Composer);
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
    let scenario = Scenario::given(RAW, ThreadState::default());
    assert_eq!(
        review::bindings(&scenario.model.review, input(Key::Char('q'))),
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
        review::bindings(&scenario.model.review, input(Key::Other)),
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
        rows: 3,
        columns: 80,
    });
    scenario.when_event(review::Event::ScrollViewport(1));

    assert_eq!(scenario.model.review.scroll(), 3);
}

#[test]
fn long_review_reports_sticky_context_and_complete_position_at_top_middle_and_end() {
    let mut scenario = Scenario::given(LONG_DIFF, ThreadState::default());
    scenario.when_event(global::Event::ViewportResized {
        rows: 7,
        columns: 80,
    });
    let (top, _, _) = review_geometry(&scenario.model);
    assert_eq!(top.scroll, 0);
    assert_eq!(
        top.viewport
            .sticky_context
            .as_ref()
            .map(|context| context.file.as_str()),
        Some("a.rs")
    );
    assert!(top.viewport.total_rows > top.viewport.visible_rows);

    scenario.when_event(review::Event::ScrollViewport(2));
    let (middle, _, _) = review_geometry(&scenario.model);
    let maximum = middle
        .viewport
        .total_rows
        .saturating_sub(middle.viewport.visible_rows);
    assert!(middle.scroll > 0);
    assert!(middle.scroll < maximum);
    assert!(middle.viewport.sticky_context.is_some());
    let focused = scenario
        .model
        .review
        .selected_location()
        .expect("scrolling selects the hunk at the viewport top");
    let context = middle.viewport.sticky_context.as_ref().unwrap();
    assert_eq!(focused.path(), context.file);
    assert_eq!(Some(focused.hunk_header()), context.hunk_header.as_deref());

    scenario.when_event(review::Event::JumpToDiffEdge { end: true });
    let (end, _, _) = review_geometry(&scenario.model);
    assert_eq!(
        end.scroll,
        end.viewport
            .total_rows
            .saturating_sub(end.viewport.visible_rows)
    );
    assert_eq!(
        end.viewport
            .sticky_context
            .as_ref()
            .map(|context| context.file.as_str()),
        Some("c.rs")
    );
    assert_eq!(
        scenario
            .model
            .review
            .selected_location()
            .as_ref()
            .map(|location| location.path()),
        Some("c.rs")
    );
}

#[test]
fn logical_diff_position_reaches_beyond_terminal_coordinate_limits() {
    let mut raw = String::from(
        "diff --git a/huge.rs b/huge.rs\n--- a/huge.rs\n+++ b/huge.rs\n@@ -1,70000 +1,70000 @@\n",
    );
    for _ in 0..70_000 {
        raw.push_str(" unchanged\n");
    }
    raw.push_str("@@ -80000 +80000 @@ target\n-old target\n+new target\n");
    let mut scenario = Scenario::given(&raw, ThreadState::default());
    scenario.when_event(global::Event::ViewportResized {
        rows: 10,
        columns: 80,
    });
    scenario.when_event(review::Event::MoveHunk(1));

    let (targeted, _, target_rows) = review_geometry(&scenario.model);
    let target = target_rows
        .selected_target_row()
        .expect("large direct jump resolves a logical target row");
    assert!(targeted.scroll > usize::from(u16::MAX));
    assert!(target >= targeted.scroll);
    assert!(target < targeted.scroll + targeted.viewport.visible_rows);

    scenario.when_event(review::Event::JumpToDiffEdge { end: false });
    scenario.when_event(review::Event::JumpToDiffEdge { end: true });

    let (body, _, _) = review_geometry(&scenario.model);
    assert!(body.scroll > usize::from(u16::MAX));
    assert_eq!(
        body.scroll,
        body.viewport
            .total_rows
            .saturating_sub(body.viewport.visible_rows)
    );
    scenario.when_event(review::Event::ScrollRows(-3));
    let (before_end, _, _) = review_geometry(&scenario.model);
    assert_eq!(before_end.scroll, body.scroll.saturating_sub(3));
}

#[test]
fn hunk_jumps_center_small_hunks_and_start_large_hunks() {
    let mut scenario = Scenario::given(LONG_DIFF, ThreadState::default());
    scenario.when_event(global::Event::ViewportResized {
        rows: 6,
        columns: 80,
    });
    scenario.when_event(review::Event::MoveFile(1));

    let (body, _, rows) = review_geometry(&scenario.model);
    let selected = rows
        .selected_target_row()
        .expect("file jump keeps a selected hunk");
    assert_eq!(body.scroll, rows.selected_file_start_row().unwrap());
    assert!(selected >= body.scroll);
    assert!(selected < body.scroll + body.viewport.visible_rows);
    assert_eq!(
        scenario.model.review.selected_location(),
        Some(HunkLocation::new("b.rs", "@@ -40,6 +40,6 @@ middle"))
    );

    scenario.when_event(review::Event::MoveHunk(-1));
    let (previous, _, previous_rows) = review_geometry(&scenario.model);
    let previous_target = previous_rows
        .selected_target_row()
        .expect("hunk jump keeps a selected hunk");
    assert!(previous_target >= previous.scroll);
    assert!(previous_target < previous.scroll + previous.viewport.visible_rows);

    scenario.when_event(review::Event::MoveHunk(1));
    let (returned, _, returned_rows) = review_geometry(&scenario.model);
    let range = returned_rows
        .selected_hunk_range()
        .expect("hunk jump selects a physical hunk range");
    let extent = range.end.saturating_sub(range.start);
    let expected = if extent > returned.viewport.visible_rows {
        range.start
    } else {
        range
            .start
            .saturating_sub((returned.viewport.visible_rows - extent) / 2)
    }
    .min(
        returned
            .viewport
            .total_rows
            .saturating_sub(returned.viewport.visible_rows),
    );
    assert_eq!(returned.scroll, expected);
}

#[test]
fn hunkless_file_jump_reveals_its_header_and_hunk_navigation_continues() {
    let raw = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-a\n+b\ndiff --git a/image.bin b/image.bin\nBinary files a/image.bin and b/image.bin differ\ndiff --git a/c.rs b/c.rs\n--- a/c.rs\n+++ b/c.rs\n@@ -3 +3 @@\n-c\n+d\n";
    let mut scenario = Scenario::given(raw, ThreadState::default());
    scenario.when_event(global::Event::ViewportResized {
        rows: 4,
        columns: 64,
    });
    scenario.when_event(review::Event::MoveFile(1));

    let (body, _, rows) = review_geometry(&scenario.model);
    let file_header = rows
        .selected_target_row()
        .expect("hunkless selected file has a physical header target");
    assert_eq!(body.scroll, rows.selected_file_start_row().unwrap());
    assert!(file_header >= body.scroll);
    assert!(file_header < body.scroll + body.viewport.visible_rows);
    assert_eq!(
        super::view(&scenario.model)
            .file_rail
            .as_ref()
            .and_then(|rail| rail.selected),
        Some(1)
    );

    scenario.when_event(review::Event::MoveHunk(1));
    assert_eq!(
        scenario.model.review.selected_location(),
        Some(HunkLocation::new("c.rs", "@@ -3 +3 @@"))
    );
}

#[test]
fn file_jumps_align_headers_to_the_top_and_clamp_at_the_document_end() {
    let mut scenario = Scenario::given(LONG_DIFF, ThreadState::default());
    scenario.when_event(global::Event::ViewportResized {
        rows: 6,
        columns: 80,
    });

    scenario.when_event(review::Event::MoveFile(1));
    let (middle, _, middle_rows) = review_geometry(&scenario.model);
    assert_eq!(
        middle.scroll,
        middle_rows.selected_file_start_row().unwrap()
    );

    scenario.when_event(review::Event::MoveFile(-1));
    let (first, _, first_rows) = review_geometry(&scenario.model);
    assert_eq!(first.scroll, first_rows.selected_file_start_row().unwrap());

    scenario.when_event(review::Event::MoveFile(-1));
    let (last, _, last_rows) = review_geometry(&scenario.model);
    let maximum = last
        .viewport
        .total_rows
        .saturating_sub(last.viewport.visible_rows);
    assert_eq!(
        last.scroll,
        last_rows.selected_file_start_row().unwrap().min(maximum)
    );
}

#[test]
fn rollup_jump_reveals_its_thread_target_on_the_first_review_frame() {
    let mut state = ThreadState::default();
    let id = state.post(
        Anchor::new(
            "deadbeef",
            HunkLocation::new("c.rs", "@@ -80,5 +80,5 @@ last"),
        ),
        Participant {
            id: "human".into(),
            kind: ParticipantKind::Human,
        },
        "last file".into(),
        1,
    );
    let mut scenario = Scenario::given(LONG_DIFF, state);
    scenario.when_event(global::Event::ViewportResized {
        rows: 6,
        columns: 80,
    });
    scenario.when_event(review::Event::ShowRollup);
    scenario.when_event(rollup::Event::OpenSelected);
    scenario.inject(
        ActiveMode::Rollup,
        Outcome::ThreadResolved {
            id,
            result: Ok(HunkLocation::new("c.rs", "@@ -80,5 +80,5 @@ last")),
        },
    );

    assert_eq!(scenario.model.active_mode, ActiveMode::Review);
    let (body, _, rows) = review_geometry(&scenario.model);
    let selected = rows
        .selected_target_row()
        .expect("rollup jump selects a review hunk");
    assert!(selected >= body.scroll);
    assert!(selected < body.scroll + body.viewport.visible_rows);
    assert_eq!(scenario.model.review.focus(), FocusArea::Threads);
}

#[test]
fn inline_thread_jumps_reveal_each_selected_card() {
    let mut state = ThreadState::default();
    let human = Participant {
        id: "human".into(),
        kind: ParticipantKind::Human,
    };
    let location = HunkLocation::new("b.rs", "@@ -40,6 +40,6 @@ middle");
    state.post(
        Anchor::new("deadbeef", location.clone()),
        human.clone(),
        "first".into(),
        1,
    );
    state.post(Anchor::new("deadbeef", location), human, "second".into(), 2);
    let mut scenario = Scenario::given(LONG_DIFF, state);
    scenario.when_event(global::Event::ViewportResized {
        rows: 5,
        columns: 64,
    });
    scenario.when_event(review::Event::MoveFile(1));

    for expected in [0, 1] {
        scenario.when_event(review::Event::MoveThread(1));
        let (body, _, rows) = review_geometry(&scenario.model);
        let target = rows
            .selected_target_row()
            .expect("thread jump produces a physical target row");
        assert!(target >= body.scroll);
        assert!(target < body.scroll + body.viewport.visible_rows);
        assert_eq!(
            scenario.model.review.session().cursor().selected_thread(),
            expected
        );
    }
}

#[test]
fn resize_and_presentation_toggles_preserve_selection_and_viewport_anchor() {
    let mut scenario = Scenario::given(LONG_DIFF, ThreadState::default());
    scenario.when_event(global::Event::ViewportResized {
        rows: 8,
        columns: 120,
    });
    scenario.when_event(review::Event::MoveFile(1));
    scenario.when_event(review::Event::ScrollRows(2));
    let selection = scenario.model.review.selected_location();
    for columns in [88, 64, 120] {
        scenario.when_event(global::Event::ViewportResized { rows: 8, columns });
        assert_eq!(scenario.model.review.selected_location(), selection);
    }

    for event in [
        review::Event::SetLayout(LayoutMode::Stack),
        review::Event::ToggleWrap,
        review::Event::ToggleHunkHeaders,
        review::Event::ToggleSidebar,
        review::Event::SetLayout(LayoutMode::Split),
    ] {
        scenario.when_event(event);
        assert_eq!(scenario.model.review.selected_location(), selection);
    }
}

#[test]
fn scrolling_follows_the_hunk_at_the_viewport_center() {
    let raw = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,8 +1,8 @@ first\n one\n two\n three\n four\n five\n six\n seven\n-eight\n+eight changed\n@@ -20 +20 @@ second\n-old second\n+new second\n";
    let mut scenario = Scenario::given(raw, ThreadState::default());
    scenario.when_event(global::Event::ViewportResized {
        rows: 6,
        columns: 80,
    });
    scenario.when_event(review::Event::ScrollRows(11));

    assert_eq!(
        scenario
            .model
            .review
            .selected_location()
            .as_ref()
            .map(HunkLocation::hunk_header),
        Some("@@ -20 +20 @@ second")
    );
}

#[test]
fn height_shrink_and_wrapping_keep_a_previously_visible_target_visible() {
    let raw = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n+new\n@@ -20 +20 @@\n-old target\n+new target\n";
    let mut scenario = Scenario::given(raw, ThreadState::default());
    scenario.when_event(global::Event::ViewportResized {
        rows: 11,
        columns: 80,
    });
    scenario.when_event(review::Event::SetLayout(LayoutMode::Stack));
    scenario.when_event(review::Event::MoveHunk(1));

    scenario.when_event(global::Event::ViewportResized {
        rows: 6,
        columns: 80,
    });
    for event in [review::Event::ToggleWrap, review::Event::ToggleSidebar] {
        scenario.when_event(event);
        let (body, _, rows) = review_geometry(&scenario.model);
        let target = rows
            .selected_target_row()
            .expect("geometry transition retains a selected target");
        assert!(target >= body.scroll);
        assert!(target < body.scroll + body.viewport.visible_rows);
    }
}

#[test]
fn end_relative_height_shrink_keeps_a_previously_visible_target_visible() {
    let raw = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,4 +1,4 @@\n-old one\n+new one\n context two\n context three\n context four\n";
    let mut scenario = Scenario::given(raw, ThreadState::default());
    scenario.when_event(global::Event::ViewportResized {
        rows: 12,
        columns: 80,
    });
    scenario.when_event(review::Event::JumpToDiffEdge { end: true });
    let (before, _, rows) = review_geometry(&scenario.model);
    let target = rows
        .selected_target_row()
        .expect("review has a selected hunk target");
    assert!(target >= before.scroll);
    assert!(target < before.scroll + before.viewport.visible_rows);

    scenario.when_event(global::Event::ViewportResized {
        rows: 5,
        columns: 80,
    });
    let (after, _, rows) = review_geometry(&scenario.model);
    let target = rows
        .selected_target_row()
        .expect("review retains its selected hunk target");
    assert!(target >= after.scroll);
    assert!(target < after.scroll + after.viewport.visible_rows);

    scenario.when_event(review::Event::ToggleWrap);
    let (wrapped, _, rows) = review_geometry(&scenario.model);
    let target = rows
        .selected_target_row()
        .expect("review retains its target after wrapping");
    assert!(target >= wrapped.scroll);
    assert!(target < wrapped.scroll + wrapped.viewport.visible_rows);
}

#[test]
fn end_relative_and_transient_mode_transitions_preserve_review_position() {
    let mut scenario = Scenario::given(LONG_DIFF, ThreadState::default());
    scenario.when_event(global::Event::ViewportResized {
        rows: 7,
        columns: 80,
    });
    scenario.when_event(review::Event::JumpToDiffEdge { end: true });

    for event in [
        review::Event::SetLayout(LayoutMode::Stack),
        review::Event::ToggleWrap,
        review::Event::ToggleSidebar,
    ] {
        scenario.when_event(event);
        let (body, _, _) = review_geometry(&scenario.model);
        assert_eq!(
            body.scroll,
            body.viewport
                .total_rows
                .saturating_sub(body.viewport.visible_rows)
        );
    }
    scenario.when_event(global::Event::ViewportResized {
        rows: 10,
        columns: 120,
    });
    let (at_end, _, _) = review_geometry(&scenario.model);
    assert_eq!(
        at_end.scroll,
        at_end
            .viewport
            .total_rows
            .saturating_sub(at_end.viewport.visible_rows)
    );

    scenario.when_event(review::Event::ScrollRows(-2));
    let (before_modes, _, _) = review_geometry(&scenario.model);
    scenario.when_event(review::Event::BeginThread { always_new: true });
    scenario.when_event(composer::Event::Cancel);
    scenario.when_event(global::Event::OpenHelp);
    scenario.when_event(help::Event::Close);
    scenario.when_event(review::Event::ShowRollup);
    scenario.when_event(rollup::Event::Close);
    let (after_modes, _, _) = review_geometry(&scenario.model);
    assert_eq!(after_modes.scroll, before_modes.scroll);
}

#[test]
fn reload_and_context_adjustment_keep_the_closest_location_and_clamp_geometry() {
    let mut scenario = Scenario::given(LONG_DIFF, ThreadState::default());
    scenario.when_event(global::Event::ViewportResized {
        rows: 6,
        columns: 64,
    });
    scenario.when_event(review::Event::MoveFile(1));
    scenario.when_event(review::Event::ScrollRows(3));
    scenario.when_event(review::Event::AdjustContext(1));
    scenario.inject(
        ActiveMode::Review,
        Outcome::DiffReloaded {
            purpose: review::ReloadPurpose::ContextChanged,
            result: Ok(LoadedDiff {
                text: RELOADED_LONG_DIFF.into(),
                document: DiffDocument::parse(RELOADED_LONG_DIFF),
            }),
        },
    );

    assert_eq!(
        scenario.model.review.selected_location(),
        Some(HunkLocation::new("b.rs", "@@ -42,4 +42,4 @@ middle"))
    );
    let (body, _, rows) = review_geometry(&scenario.model);
    let selected = rows
        .selected_target_row()
        .expect("reloaded snapshot retains a selected hunk");
    assert!(selected >= body.scroll);
    assert!(selected < body.scroll + body.viewport.visible_rows);
    assert!(
        body.scroll
            <= body
                .viewport
                .total_rows
                .saturating_sub(body.viewport.visible_rows)
    );
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
    assert!(matches!(view.overlay, Some(Overlay::Composer(_))));
}

#[test]
fn scenario_trace_uses_virtual_time_without_wall_clock_dependency() {
    let mut scenario = Scenario::given(RAW, ThreadState::default());
    scenario.advance_virtual_time(250);
    scenario.when_event(review::Event::ScrollRows(1));
    assert_eq!(scenario.virtual_time_ms, 250);
    assert_eq!(scenario.model.review.scroll(), 0);
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
        Some("cancelled comment draft")
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
    assert_eq!(scenario.model.active_mode, ActiveMode::Composer);
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
fn one_review_cursor_drives_file_hunk_thread_and_diff_targets() {
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

    // A thread target is left by returning to the diff, not by continuing on
    // to the file rail.
    scenario.when_input(input(Key::Tab));
    assert_eq!(scenario.model.review.focus(), FocusArea::Review);
    scenario.when_event(global::Event::ViewportResized {
        rows: 3,
        columns: 80,
    });
    let previous_scroll = scenario.model.review.scroll();
    scenario.when_input(input(Key::Char('j')));
    assert_eq!(scenario.model.review.scroll(), previous_scroll + 1);
    assert!(
        scenario
            .model
            .global
            .status
            .as_deref()
            .is_some_and(|status| status == format!("diff row {}", previous_scroll + 2))
    );
}

#[test]
fn lifecycle_cards_fold_deterministically_and_actions_follow_the_visible_target() {
    let mut scenario = Scenario::given(RAW, lifecycle_threads());
    scenario.when_event(global::Event::ViewportResized {
        rows: 12,
        columns: 56,
    });

    let view = super::view(&scenario.model);
    let Body::Review(review) = view.body else {
        panic!("review body")
    };
    let cards = &review.files[0].hunks[0].threads;
    assert_eq!(
        cards
            .iter()
            .map(|card| card.id.to_string())
            .collect::<Vec<_>>(),
        ["0", "1", "2"]
    );
    assert_eq!(
        presentation::thread_card_rows(&cards[1], 54, false).len(),
        1
    );
    assert!(presentation::thread_card_rows(&cards[2], 54, true).len() > 1);

    scenario.when_input(input(Key::Char('t')));
    scenario.when_input(input(Key::Char('t')));
    let view = super::view(&scenario.model);
    let Body::Review(review) = view.body else {
        panic!("review body")
    };
    assert!(review.files[0].hunks[0].threads[1].active);
    assert!(
        presentation::thread_card_rows(&review.files[0].hunks[0].threads[1], 54, false).len() > 1
    );

    scenario.when_input(input(Key::Char('e')));
    scenario.when_input(input(Key::Tab));
    let view = super::view(&scenario.model);
    let Body::Review(review) = view.body else {
        panic!("review body")
    };
    assert!(!review.files[0].hunks[0].threads[1].active);
    assert!(review.files[0].hunks[0].threads[1].expanded);

    scenario.when_input(input(Key::Char('t')));
    scenario.when_input(input(Key::Char('e')));
    scenario.when_input(input(Key::Tab));
    let view = super::view(&scenario.model);
    let Body::Review(review) = view.body else {
        panic!("review body")
    };
    assert_eq!(
        presentation::thread_card_rows(&review.files[0].hunks[0].threads[1], 54, false).len(),
        1
    );

    scenario.when_input(input(Key::Char('t')));
    scenario.when_input(input(Key::Char('R')));
    let effects = scenario.trace.iter().rev().find_map(|trace| match trace {
        Trace::Effect(effect) => Some(effect),
        Trace::Event(_) => None,
    });
    assert!(matches!(
        effects,
        Some(Effect::ChangeThreads {
            operation: crate::thread::ThreadOperation::Reopen { id },
            ..
        }) if id.to_string() == "1"
    ));
}

#[test]
fn contextual_keys_follow_mode_and_visible_thread_availability() {
    let mut empty = Scenario::given(RAW, ThreadState::default());
    let view = super::view(&empty.model);
    assert!(view.footer.contextual_keys.text.contains("c comment"));
    assert!(!view.footer.contextual_keys.text.contains("x resolve"));
    // With no threads to address, Tab steps over the thread region and lands on
    // the file rail instead of refusing to move.
    empty.when_input(input(Key::Tab));
    assert_eq!(empty.model.review.focus(), FocusArea::Files);
    assert_eq!(
        empty.model.global.status.as_deref(),
        Some("file rail focused")
    );
    empty.when_input(input(Key::Tab));
    assert_eq!(empty.model.review.focus(), FocusArea::Review);

    empty.when_input(input(Key::Char('c')));
    let view = super::view(&empty.model);
    assert_eq!(
        view.footer.contextual_keys.text,
        "Esc cancel · Ctrl-J post · Enter newline"
    );
    empty.when_input(input(Key::Esc));
    empty.when_input(input(Key::Char('?')));
    let view = super::view(&empty.model);
    assert!(view.footer.contextual_keys.text.starts_with("Esc/? close"));
    empty.when_input(input(Key::Esc));
    empty.when_input(input(Key::Char('v')));
    let view = super::view(&empty.model);
    assert!(view.footer.contextual_keys.text.contains("no targets"));
    assert!(
        view.footer
            .current_context
            .text
            .contains("Target: no threads")
    );
}

#[test]
fn composer_edits_multiline_unicode_at_a_real_cursor_and_confirms_discard() {
    let mut scenario = Scenario::given(RAW, ThreadState::default());
    scenario.when_input(input(Key::Char('c')));
    for character in "ab画".chars() {
        scenario.when_input(input(Key::Char(character)));
    }
    scenario.when_input(input(Key::Left));
    scenario.when_input(input(Key::Char('X')));
    scenario.when_input(input(Key::Enter));
    assert_eq!(scenario.model.composer.input(), "abX\n画");
    assert_eq!(scenario.model.composer.cursor(), "abX\n".len());

    scenario.when_input(input(Key::Up));
    scenario.when_input(input(Key::Home));
    scenario.when_input(input(Key::Delete));
    assert_eq!(scenario.model.composer.input(), "bX\n画");
    scenario.when_input(input(Key::End));
    scenario.when_input(input(Key::DeleteWordBackward));
    assert_eq!(scenario.model.composer.input(), "\n画");

    scenario.when_input(input(Key::Esc));
    assert_eq!(scenario.model.active_mode, ActiveMode::Composer);
    assert!(matches!(
        super::view(&scenario.model).overlay,
        Some(Overlay::Composer(ref composer))
            if composer.message.as_deref()
                == Some("Unsaved draft. Press Esc again to discard it.")
    ));
    scenario.when_input(input(Key::Esc));
    assert_eq!(scenario.model.active_mode, ActiveMode::Review);
    assert_eq!(scenario.model.composer.input(), "");
}

#[test]
fn composer_keeps_empty_and_failed_submissions_open_for_retry() {
    let mut scenario = Scenario::given(RAW, threads());
    scenario.when_input(input(Key::Char('t')));
    let reply_to = scenario
        .model
        .review
        .selected_thread_id(&scenario.model.global.threads);
    scenario.when_input(input(Key::Char('c')));
    assert_eq!(scenario.model.composer.reply_to(), reply_to);

    scenario.when_input(input(Key::Submit));
    assert_eq!(scenario.model.active_mode, ActiveMode::Composer);
    assert_eq!(scenario.model.global.pending, None);
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("comment cannot be empty")
    );

    for character in "retry this 画面".chars() {
        scenario.when_input(input(Key::Char(character)));
    }
    scenario.when_input(input(Key::Submit));
    assert_eq!(
        scenario.model.global.pending.map(|pending| pending.kind),
        Some(PendingEffectKind::ChangeThreads)
    );
    assert_eq!(scenario.model.active_mode, ActiveMode::Composer);
    assert_eq!(scenario.model.composer.input(), "retry this 画面");
    scenario.inject(
        ActiveMode::Composer,
        Outcome::ThreadsChanged {
            result: Err("could not post thread: anchor unavailable".into()),
        },
    );
    assert_eq!(scenario.model.active_mode, ActiveMode::Composer);
    assert_eq!(scenario.model.composer.input(), "retry this 画面");
    assert_eq!(scenario.model.composer.reply_to(), reply_to);

    scenario.when_input(input(Key::Submit));
    scenario.inject(
        ActiveMode::Composer,
        Outcome::ThreadsChanged {
            result: Ok(ThreadChange {
                state: scenario.model.global.threads.clone(),
                success: ThreadSuccess::Replied,
            }),
        },
    );
    assert_eq!(scenario.model.active_mode, ActiveMode::Review);
    assert_eq!(scenario.model.composer.input(), "");
    assert_eq!(scenario.model.composer.reply_to(), None);
    assert!(matches!(
        super::view(&scenario.model).overlay,
        Some(Overlay::Thread(ref thread)) if Some(thread.id) == reply_to
    ));
    assert_eq!(
        scenario.model.global.status.as_deref(),
        Some("posted reply")
    );
}

#[test]
fn composer_grows_then_scrolls_and_reflows_across_narrow_resizes() {
    let mut scenario = Scenario::given(RAW, ThreadState::default());
    scenario.when_event(global::Event::ViewportResized {
        rows: 6,
        columns: 48,
    });
    scenario.when_input(input(Key::Char('c')));
    for character in "one 画面 two three four five six seven\neight\nnine\nten".chars() {
        scenario.when_input(input(Key::Char(character)));
    }
    let Some(Overlay::Composer(narrow)) = super::view(&scenario.model).overlay else {
        panic!("composer overlay")
    };
    assert!(narrow.height <= 6);
    assert!(narrow.scroll > 0);
    assert!(narrow.cursor_row >= narrow.scroll);

    scenario.when_event(global::Event::ViewportResized {
        rows: 16,
        columns: 120,
    });
    let Some(Overlay::Composer(wide)) = super::view(&scenario.model).overlay else {
        panic!("composer overlay")
    };
    assert!(wide.height <= 12);
    assert!(wide.lines.len() < narrow.lines.len());
    assert!(wide.cursor_row >= wide.scroll);
    assert!(wide.cursor_row < wide.scroll + usize::from(wide.height.saturating_sub(3)));
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
fn review_filters_project_semantic_state_in_git_order() {
    let (state, [a_attention, b_attention, c_attention, a_open, a_resolved]) = filter_threads();
    let mut scenario = Scenario::given(FILTER_DIFF, state);

    let (all, _, _) = review_geometry(&scenario.model);
    assert_eq!(
        all.files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        ["a.rs", "b.rs", "c.rs"]
    );
    assert_eq!(
        scenario.model.review.filter(),
        review::ReviewFilter::AllChanges
    );
    scenario.when_event(review::Event::CycleFilter(1));
    scenario.when_event(review::Event::ShowAllChanges);
    assert_eq!(
        scenario.model.review.filter(),
        review::ReviewFilter::AllChanges
    );

    scenario.when_event(review::Event::CycleFilter(1));
    let (attention, _, _) = review_geometry(&scenario.model);
    assert_eq!(
        scenario.model.review.filter(),
        review::ReviewFilter::NeedsAttention
    );
    assert_eq!(
        attention
            .files
            .iter()
            .flat_map(|file| file.hunks.iter().map(|hunk| hunk.anchor.clone()))
            .collect::<Vec<_>>(),
        [
            HunkLocation::new("a.rs", "@@ -1 +1 @@ first"),
            HunkLocation::new("b.rs", "@@ -1 +1 @@ only"),
            HunkLocation::new("c.rs", "@@ -1 +1 @@ only"),
        ]
    );
    assert_eq!(
        attention
            .files
            .iter()
            .flat_map(|file| &file.hunks)
            .flat_map(|hunk| &hunk.threads)
            .map(|thread| thread.id)
            .collect::<Vec<_>>(),
        [a_attention, b_attention, c_attention]
    );

    scenario.when_event(review::Event::CycleFilter(1));
    let (open, _, _) = review_geometry(&scenario.model);
    assert_eq!(
        scenario.model.review.filter(),
        review::ReviewFilter::OpenThreads
    );
    assert_eq!(
        open.files
            .iter()
            .flat_map(|file| &file.hunks)
            .flat_map(|hunk| &hunk.threads)
            .map(|thread| thread.id)
            .collect::<Vec<_>>(),
        [a_attention, a_open, c_attention]
    );

    scenario.when_event(review::Event::CycleFilter(1));
    let (threaded, _, _) = review_geometry(&scenario.model);
    assert_eq!(
        scenario.model.review.filter(),
        review::ReviewFilter::ThreadedHunks
    );
    assert_eq!(
        threaded
            .files
            .iter()
            .flat_map(|file| &file.hunks)
            .map(|hunk| hunk.anchor.clone())
            .collect::<Vec<_>>(),
        [
            HunkLocation::new("a.rs", "@@ -1 +1 @@ first"),
            HunkLocation::new("a.rs", "@@ -10 +10 @@ second"),
            HunkLocation::new("b.rs", "@@ -1 +1 @@ only"),
            HunkLocation::new("c.rs", "@@ -1 +1 @@ only"),
        ]
    );
    assert!(
        threaded.files[0].hunks[0]
            .threads
            .iter()
            .any(|thread| thread.id == a_resolved)
    );

    scenario.when_event(review::Event::CycleFilter(1));
    assert_eq!(
        scenario.model.review.filter(),
        review::ReviewFilter::AllChanges
    );
    assert_eq!(
        super::view(&scenario.model).header.active_filter,
        "All changes"
    );
}

#[test]
fn filtered_empty_view_exposes_no_hidden_target_or_composer_action() {
    let mut scenario = Scenario::given(FILTER_DIFF, ThreadState::default());
    let raw_location = scenario.model.review.selected_location();
    scenario.when_event(review::Event::CycleFilter(1));
    let view = super::view(&scenario.model);
    let Body::Review(body) = view.body else {
        panic!("review mode must render the review body");
    };
    assert!(body.files.is_empty());
    assert!(body.empty_state.is_some());
    assert!(!view.footer.current_context.text.contains("Target:"));
    assert!(
        scenario
            .model
            .review
            .projected_location(&scenario.model.global.threads)
            .is_none()
    );
    assert_eq!(scenario.model.review.selected_location(), raw_location);

    let effects = scenario.when_event(review::Event::BeginThread { always_new: true });
    assert!(effects.is_empty());
    assert_eq!(scenario.model.active_mode, ActiveMode::Review);
    assert_eq!(scenario.model.global.pending, None);
    assert_eq!(scenario.model.review.selected_location(), raw_location);
    assert!(
        scenario
            .model
            .global
            .status
            .as_deref()
            .is_some_and(|status| {
                status.contains("no target in Needs attention")
                    && status.contains("A for All changes")
            })
    );
}

#[test]
fn attention_traversal_uses_git_order_and_wraps_at_both_ends() {
    let (state, [a_attention, b_attention, c_attention, _, _]) = filter_threads();
    let mut scenario = Scenario::given(FILTER_DIFF, state);
    scenario.when_event(review::Event::MoveHunk(1));

    for (id, location) in [
        (a_attention, HunkLocation::new("a.rs", "@@ -1 +1 @@ first")),
        (b_attention, HunkLocation::new("b.rs", "@@ -1 +1 @@ only")),
        (c_attention, HunkLocation::new("c.rs", "@@ -1 +1 @@ only")),
        (a_attention, HunkLocation::new("a.rs", "@@ -1 +1 @@ first")),
    ] {
        let effects = scenario.when_event(review::Event::MoveAttention(1));
        assert!(matches!(
            effects.as_slice(),
            [Effect::ResolveThread {
                owner: ActiveMode::Review,
                id: emitted,
                ..
            }] if *emitted == id
        ));
        scenario.inject(
            ActiveMode::Review,
            Outcome::ThreadResolved {
                id,
                result: Ok(location),
            },
        );
        assert_eq!(
            scenario
                .model
                .review
                .selected_thread_id(&scenario.model.global.threads),
            Some(id)
        );
    }

    let effects = scenario.when_event(review::Event::MoveAttention(-1));
    assert!(matches!(
        effects.as_slice(),
        [Effect::ResolveThread {
            owner: ActiveMode::Review,
            id,
            ..
        }] if *id == c_attention
    ));
    scenario.inject(
        ActiveMode::Review,
        Outcome::ThreadResolved {
            id: c_attention,
            result: Ok(HunkLocation::new("c.rs", "@@ -1 +1 @@ only")),
        },
    );
    assert_eq!(
        scenario
            .model
            .review
            .selected_thread_id(&scenario.model.global.threads),
        Some(c_attention)
    );

    let mut no_attention = Scenario::given(FILTER_DIFF, ThreadState::default());
    no_attention.when_event(review::Event::MoveAttention(1));
    assert_eq!(
        no_attention.model.global.status.as_deref(),
        Some("no needs-attention threads")
    );
}

#[test]
fn mutation_reconciles_a_filtered_target_against_the_replacement_state() {
    let (state, [_, b_attention, c_attention, _, _]) = filter_threads();
    let mut scenario = Scenario::given(FILTER_DIFF, state);
    scenario.when_event(review::Event::CycleFilter(1));
    scenario
        .model
        .review
        .select_thread_location(
            b_attention,
            &HunkLocation::new("b.rs", "@@ -1 +1 @@ only"),
            &scenario.model.global.threads,
        )
        .unwrap();
    scenario.when_event(review::Event::ToggleAttention);
    let mut changed = scenario.model.global.threads.clone();
    changed.set_needs_attention(b_attention, false).unwrap();
    scenario.inject(
        ActiveMode::Review,
        Outcome::ThreadsChanged {
            result: Ok(ThreadChange {
                state: changed,
                success: ThreadSuccess::AttentionToggled,
            }),
        },
    );

    assert_eq!(
        scenario.model.review.filter(),
        review::ReviewFilter::NeedsAttention
    );
    assert_eq!(
        scenario.model.review.selected_location(),
        Some(HunkLocation::new("c.rs", "@@ -1 +1 @@ only"))
    );
    assert_eq!(
        scenario
            .model
            .review
            .selected_thread_id(&scenario.model.global.threads),
        Some(c_attention)
    );
    let (body, _, rows) = review_geometry(&scenario.model);
    let selected = rows.selected_target_row().unwrap();
    assert!(selected >= body.scroll && selected < body.scroll + body.viewport.visible_rows);
}

#[test]
fn resolving_an_open_filtered_thread_keeps_the_closest_visible_target() {
    let (state, [a_attention, _, _, a_open, _]) = filter_threads();
    let mut scenario = Scenario::given(FILTER_DIFF, state);
    scenario.when_event(review::Event::CycleFilter(1));
    scenario.when_event(review::Event::CycleFilter(1));
    scenario
        .model
        .review
        .select_thread_location(
            a_open,
            &HunkLocation::new("a.rs", "@@ -10 +10 @@ second"),
            &scenario.model.global.threads,
        )
        .unwrap();
    scenario.when_event(review::Event::CloseThread);
    let mut changed = scenario.model.global.threads.clone();
    changed
        .close(
            a_open,
            &Participant {
                id: "reviewer".into(),
                kind: ParticipantKind::Human,
            },
        )
        .unwrap();
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
        scenario.model.review.filter(),
        review::ReviewFilter::OpenThreads
    );
    assert_eq!(
        scenario.model.review.selected_location(),
        Some(HunkLocation::new("a.rs", "@@ -1 +1 @@ first"))
    );
    assert_eq!(
        scenario
            .model
            .review
            .selected_thread_id(&scenario.model.global.threads),
        Some(a_attention)
    );
}

#[test]
fn rollup_jump_resets_a_hiding_filter_and_reveals_the_exact_thread() {
    let (state, [_, b_attention, _, _, _]) = filter_threads();
    let mut scenario = Scenario::given(FILTER_DIFF, state);
    scenario.when_event(review::Event::CycleFilter(1));
    scenario.when_event(review::Event::CycleFilter(1));
    assert_eq!(
        scenario.model.review.filter(),
        review::ReviewFilter::OpenThreads
    );

    scenario.when_event(review::Event::ShowRollup);
    scenario.when_event(rollup::Event::OpenSelected);
    scenario.inject(
        ActiveMode::Rollup,
        Outcome::ThreadResolved {
            id: b_attention,
            result: Ok(HunkLocation::new("b.rs", "@@ -1 +1 @@ only")),
        },
    );

    assert_eq!(scenario.model.active_mode, ActiveMode::Review);
    assert_eq!(
        scenario.model.review.filter(),
        review::ReviewFilter::AllChanges
    );
    assert_eq!(
        scenario
            .model
            .review
            .selected_thread_id(&scenario.model.global.threads),
        Some(b_attention)
    );
    assert!(
        scenario
            .model
            .global
            .status
            .as_deref()
            .is_some_and(|status| status.contains("filter reset to All changes"))
    );
    let (body, _, rows) = review_geometry(&scenario.model);
    let selected = rows.selected_target_row().unwrap();
    assert!(selected >= body.scroll && selected < body.scroll + body.viewport.visible_rows);
}

#[test]
fn filter_survives_geometry_context_and_successful_reload() {
    let (state, _) = filter_threads();
    let mut scenario = Scenario::given(FILTER_DIFF, state);
    for _ in 0..3 {
        scenario.when_event(review::Event::CycleFilter(1));
    }
    scenario.when_event(global::Event::ViewportResized {
        rows: 9,
        columns: 64,
    });
    scenario.when_event(review::Event::SetLayout(LayoutMode::Stack));
    scenario.when_event(review::Event::ToggleWrap);
    scenario.when_event(review::Event::AdjustContext(1));
    scenario.inject(
        ActiveMode::Review,
        Outcome::DiffReloaded {
            purpose: review::ReloadPurpose::ContextChanged,
            result: Ok(LoadedDiff {
                text: FILTER_DIFF.into(),
                document: DiffDocument::parse(FILTER_DIFF),
            }),
        },
    );
    assert_eq!(
        scenario.model.review.filter(),
        review::ReviewFilter::ThreadedHunks
    );
    assert_eq!(
        super::view(&scenario.model).header.active_filter,
        "Threaded hunks"
    );
    scenario.when_event(review::Event::ReloadDiff);
    scenario.inject(
        ActiveMode::Review,
        Outcome::DiffReloaded {
            purpose: review::ReloadPurpose::Manual,
            result: Err("repository unavailable".into()),
        },
    );
    assert_eq!(
        scenario.model.review.filter(),
        review::ReviewFilter::ThreadedHunks
    );
}

#[test]
fn context_reload_resolves_the_immutable_anchor_to_the_new_hunk_header() {
    let old_location = HunkLocation::new("context.rs", "@@ -30,6 +30,49 @@ fn context()");
    let new_location = HunkLocation::new("context.rs", "@@ -29,8 +29,51 @@ fn context()");
    let mut state = ThreadState::default();
    let id = state.post(
        Anchor::new("deadbeef", old_location.clone()),
        Participant {
            id: "reviewer".into(),
            kind: ParticipantKind::Human,
        },
        "survives context change".into(),
        1,
    );
    state.set_needs_attention(id, true).unwrap();
    let mut scenario = Scenario::given(CONTEXT_U3_DIFF, state);
    scenario.when_event(review::Event::CycleFilter(1));
    scenario.when_event(review::Event::MoveThread(1));
    scenario.when_event(review::Event::AdjustContext(1));
    scenario.inject(
        ActiveMode::Review,
        Outcome::DiffReloaded {
            purpose: review::ReloadPurpose::ContextChanged,
            result: Ok(LoadedDiff {
                text: CONTEXT_U4_DIFF.into(),
                document: DiffDocument::parse(CONTEXT_U4_DIFF),
            }),
        },
    );

    assert_eq!(
        scenario.model.review.selected_location(),
        Some(new_location.clone())
    );
    assert_eq!(scenario.model.review.focus(), FocusArea::Threads);
    assert_eq!(
        scenario
            .model
            .review
            .selected_thread_id(&scenario.model.global.threads),
        Some(id)
    );
    let (body, _, rows) = review_geometry(&scenario.model);
    assert_eq!(body.files[0].hunks[0].threads[0].id, id);
    let selected = rows.selected_target_row().unwrap();
    assert!(selected >= body.scroll && selected < body.scroll + body.viewport.visible_rows);

    let effects = scenario.when_event(review::Event::MoveAttention(1));
    assert!(matches!(
        effects.as_slice(),
        [Effect::ResolveThread { id: emitted, .. }] if *emitted == id
    ));
    scenario.inject(
        ActiveMode::Review,
        Outcome::ThreadResolved {
            id,
            result: Ok(old_location.clone()),
        },
    );
    scenario.when_event(review::Event::ShowRollup);
    let effects = scenario.when_event(rollup::Event::OpenSelected);
    assert!(matches!(
        effects.as_slice(),
        [Effect::ResolveThread { id: emitted, .. }] if *emitted == id
    ));
    scenario.inject(
        ActiveMode::Rollup,
        Outcome::ThreadResolved {
            id,
            result: Ok(old_location),
        },
    );
    assert_eq!(
        scenario.model.review.selected_location(),
        Some(new_location)
    );
    assert_eq!(
        scenario
            .model
            .review
            .selected_thread_id(&scenario.model.global.threads),
        Some(id)
    );
}

#[test]
fn ambiguous_split_hunks_choose_nearest_start_then_git_order() {
    let old_location = HunkLocation::new("split.rs", "@@ -20,20 +20,20 @@ merged");
    let mut state = ThreadState::default();
    let id = state.post(
        Anchor::new("deadbeef", old_location),
        Participant {
            id: "reviewer".into(),
            kind: ParticipantKind::Human,
        },
        "ambiguous split".into(),
        1,
    );
    let mut scenario = Scenario::given(MERGED_ANCHOR_DIFF, state);
    for _ in 0..3 {
        scenario.when_event(review::Event::CycleFilter(1));
    }
    scenario.when_event(review::Event::ReloadDiff);
    scenario.inject(
        ActiveMode::Review,
        Outcome::DiffReloaded {
            purpose: review::ReloadPurpose::Manual,
            result: Ok(LoadedDiff {
                text: SPLIT_HUNKS_DIFF.into(),
                document: DiffDocument::parse(SPLIT_HUNKS_DIFF),
            }),
        },
    );

    let (body, _, _) = review_geometry(&scenario.model);
    assert_eq!(body.files.len(), 1);
    assert_eq!(body.files[0].hunks.len(), 1);
    assert_eq!(
        body.files[0].hunks[0].anchor,
        HunkLocation::new("split.rs", "@@ -10,15 +10,15 @@ first")
    );
    assert_eq!(body.files[0].hunks[0].threads[0].id, id);
}

#[test]
fn full_diff_search_resets_filter_explicitly_and_cancel_restores_it() {
    let (state, _) = filter_threads();
    let mut scenario = Scenario::given(FILTER_DIFF, state);
    scenario.when_event(review::Event::CycleFilter(1));
    let origin = scenario.model.review.selected_location();

    type_search(&mut scenario, "old_a10");
    assert_eq!(
        scenario.model.review.filter(),
        review::ReviewFilter::AllChanges
    );
    assert!(
        scenario
            .model
            .global
            .status
            .as_deref()
            .is_some_and(|status| status.contains("full-diff search"))
    );
    scenario.when_event(review::Event::CancelSearch);
    assert_eq!(
        scenario.model.review.filter(),
        review::ReviewFilter::NeedsAttention
    );
    assert_eq!(scenario.model.review.selected_location(), origin);

    type_search(&mut scenario, "old_a10");
    scenario.when_event(review::Event::FinishSearch);
    scenario.when_event(review::Event::CycleFilter(1));
    assert_eq!(
        scenario.model.review.filter(),
        review::ReviewFilter::NeedsAttention
    );
    assert!(scenario.model.review.search_summary().is_none());
    assert!(
        scenario
            .model
            .global
            .status
            .as_deref()
            .is_some_and(|status| status.contains("cleared search"))
    );
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
    assert!(view.footer.contextual_keys.text.contains("Tab diff"));
    assert!(view.footer.contextual_keys.text.contains("x resolve"));
}
