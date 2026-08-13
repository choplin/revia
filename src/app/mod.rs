pub mod effect;
pub mod global;
#[cfg(test)]
mod scenario;

use crate::{
    diff::{DiffRequest, LoadedDiff},
    input::{BindingResolution, PhysicalInput},
    mode::{composer, help, review, rollup},
    semantic,
    thread::{Resolution, ThreadState},
};

pub use crate::mode::ActiveMode;
pub use effect::{Effect, EffectResult, Outcome};

#[derive(Debug)]
pub struct Model {
    pub global: global::Model,
    pub review: review::Model,
    pub composer: composer::Model,
    pub help: help::Model,
    pub rollup: rollup::Model,
    pub active_mode: ActiveMode,
}

impl Model {
    pub fn new(request: DiffRequest, diff: LoadedDiff, threads: ThreadState) -> Self {
        Self {
            global: global::Model::new(threads),
            review: review::Model::new(request, diff),
            composer: composer::Model::default(),
            help: help::Model::default(),
            rollup: rollup::Model::default(),
            active_mode: ActiveMode::Review,
        }
    }

    pub fn is_running(&self) -> bool {
        self.global.running == global::RunningState::Running
    }
}

/// Open dispatch protocol implemented by Global and each Mode-local Event.
///
/// This keeps `ActiveMode` as the only closed Mode sum type.
pub trait Event: std::fmt::Debug {
    fn dispatch(self, model: &mut Model) -> Vec<Effect>;
}

impl Event for global::Event {
    fn dispatch(self, model: &mut Model) -> Vec<Effect> {
        let result = global::update(&mut model.global, self);
        apply_global(model, result)
    }
}

impl Event for review::Event {
    fn dispatch(self, model: &mut Model) -> Vec<Effect> {
        if model.active_mode == ActiveMode::Review {
            update_review(model, self)
        } else {
            Vec::new()
        }
    }
}

impl Event for composer::Event {
    fn dispatch(self, model: &mut Model) -> Vec<Effect> {
        if model.active_mode == ActiveMode::Composer {
            update_composer(model, self)
        } else {
            Vec::new()
        }
    }
}

impl Event for help::Event {
    fn dispatch(self, model: &mut Model) -> Vec<Effect> {
        if model.active_mode == ActiveMode::Help {
            let result = help::update(&mut model.help, self);
            apply_help(model, result)
        } else {
            Vec::new()
        }
    }
}

impl Event for rollup::Event {
    fn dispatch(self, model: &mut Model) -> Vec<Effect> {
        if model.active_mode == ActiveMode::Rollup {
            update_rollup(model, self)
        } else {
            Vec::new()
        }
    }
}

impl Event for EffectResult {
    fn dispatch(self, model: &mut Model) -> Vec<Effect> {
        let kind = self.outcome.pending_kind();
        if !model
            .global
            .finish_pending(self.operation_id, self.owner, kind)
        {
            model.global.status = Some("ignored stale operation result".into());
            return Vec::new();
        }
        dispatch_effect_result(model, self)
    }
}

pub fn update(model: &mut Model, event: impl Event) -> Vec<Effect> {
    event.dispatch(model)
}

pub fn handle_input(
    model: &mut Model,
    input: PhysicalInput,
) -> (BindingResolution<()>, Vec<Effect>) {
    match model.active_mode {
        ActiveMode::Review => {
            handle_mode_binding(model, input, review::bindings(&model.review, input))
        }
        ActiveMode::Composer => handle_mode_binding(model, input, composer::bindings(input)),
        ActiveMode::Help => handle_mode_binding(model, input, help::bindings(input)),
        ActiveMode::Rollup => handle_mode_binding(model, input, rollup::bindings(input)),
    }
}

fn handle_mode_binding<E: Event>(
    model: &mut Model,
    input: PhysicalInput,
    resolution: BindingResolution<E>,
) -> (BindingResolution<()>, Vec<Effect>) {
    match resolution {
        BindingResolution::Delegate => apply_binding(model, global::bindings(input)),
        resolution => apply_binding(model, resolution),
    }
}

fn apply_binding<E: Event>(
    model: &mut Model,
    resolution: BindingResolution<E>,
) -> (BindingResolution<()>, Vec<Effect>) {
    match resolution {
        BindingResolution::Handle(event) => (BindingResolution::Handle(()), update(model, event)),
        BindingResolution::Override(event) => {
            (BindingResolution::Override(()), update(model, event))
        }
        BindingResolution::Delegate => (BindingResolution::Delegate, Vec::new()),
        BindingResolution::Consume => (BindingResolution::Consume, Vec::new()),
        BindingResolution::Unbound => (BindingResolution::Unbound, Vec::new()),
    }
}

fn dispatch_effect_result(model: &mut Model, result: EffectResult) -> Vec<Effect> {
    match (result.owner, result.outcome) {
        (ActiveMode::Review, Outcome::DiffReloaded { purpose, result }) => update_review(
            model,
            review::Event::EffectCompleted(review::Outcome::DiffReloaded { purpose, result }),
        ),
        (ActiveMode::Review, Outcome::ThreadsChanged { result }) => update_review(
            model,
            review::Event::EffectCompleted(review::Outcome::ThreadsChanged { result }),
        ),
        (ActiveMode::Review, Outcome::ThreadResolved { id, result }) => update_review(
            model,
            review::Event::EffectCompleted(review::Outcome::ThreadResolved { id, result }),
        ),
        (ActiveMode::Composer, Outcome::ThreadsChanged { result }) => update_composer(
            model,
            composer::Event::EffectCompleted(composer::Outcome::ThreadsChanged { result }),
        ),
        (ActiveMode::Rollup, Outcome::ThreadResolved { id, result }) => update_rollup(
            model,
            rollup::Event::EffectCompleted(rollup::Outcome::ThreadResolved { id, result }),
        ),
        (owner, outcome) => {
            debug_assert!(
                false,
                "effect outcome {outcome:?} cannot be delivered to {owner:?}"
            );
            Vec::new()
        }
    }
}

fn update_review(model: &mut Model, event: review::Event) -> Vec<Effect> {
    let result = review::update(
        &mut model.review,
        event,
        review::UpdateInput {
            threads: &model.global.threads,
            operation_pending: model.global.pending.is_some(),
        },
    );
    apply_review(model, result)
}

fn update_composer(model: &mut Model, event: composer::Event) -> Vec<Effect> {
    let input = composer::UpdateInput {
        target: model.review.request().target.clone(),
        selected_location: model.review.projected_location(&model.global.threads),
        operation_pending: model.global.pending.is_some(),
    };
    let result = composer::update(&mut model.composer, event, input);
    apply_composer(model, result)
}

fn update_rollup(model: &mut Model, event: rollup::Event) -> Vec<Effect> {
    let result = rollup::update(
        &mut model.rollup,
        event,
        rollup::UpdateInput {
            threads: &model.global.threads,
            operation_pending: model.global.pending.is_some(),
        },
    );
    apply_rollup(model, result)
}

fn apply_global(model: &mut Model, result: global::Update) -> Vec<Effect> {
    for intent in result.intents {
        match intent {
            global::Intent::OpenHelp => {
                model.help.begin(model.review.help_context());
                model.active_mode = ActiveMode::Help;
            }
            global::Intent::ResizeViewport { rows, columns } => {
                model
                    .review
                    .set_viewport(rows, columns, &model.global.threads);
                model.composer.set_viewport(rows, columns);
                model.help.set_viewport(rows, columns);
            }
        }
    }
    result
        .effects
        .into_iter()
        .map(|effect| match effect {})
        .collect()
}

fn apply_review(model: &mut Model, result: review::Update) -> Vec<Effect> {
    for intent in result.intents {
        match intent {
            review::Intent::SetStatus(status) => model.global.status = Some(status),
            review::Intent::ReplaceThreads(threads) => replace_threads(model, threads),
            review::Intent::OpenComposer { reply_to } => {
                model.composer.begin(reply_to);
                model.active_mode = ActiveMode::Composer;
            }
            review::Intent::OpenRollup => {
                model.rollup.prepare(&model.global.threads);
                model.active_mode = ActiveMode::Rollup;
            }
        }
    }
    result
        .effects
        .into_iter()
        .map(|effect| match effect {
            review::Effect::ReloadDiff { request, purpose } => {
                let operation_id = model
                    .global
                    .set_pending(ActiveMode::Review, effect::PendingEffectKind::ReloadDiff);
                Effect::ReloadDiff {
                    operation_id,
                    owner: ActiveMode::Review,
                    request,
                    purpose,
                }
            }
            review::Effect::ChangeThreads(operation) => {
                let operation_id = model
                    .global
                    .set_pending(ActiveMode::Review, effect::PendingEffectKind::ChangeThreads);
                Effect::ChangeThreads {
                    operation_id,
                    owner: ActiveMode::Review,
                    operation,
                }
            }
            review::Effect::ResolveThread { id } => {
                let operation_id = model
                    .global
                    .set_pending(ActiveMode::Review, effect::PendingEffectKind::ResolveThread);
                Effect::ResolveThread {
                    operation_id,
                    owner: ActiveMode::Review,
                    id,
                }
            }
        })
        .collect()
}

fn apply_composer(model: &mut Model, result: composer::Update) -> Vec<Effect> {
    for intent in result.intents {
        match intent {
            composer::Intent::Close => model.active_mode = ActiveMode::Review,
            composer::Intent::FocusThread(id) => {
                if !model.review.focus_thread(id, &model.global.threads) {
                    model.global.status =
                        Some(format!("posted thread #{id}, but its hunk is hidden"));
                }
            }
            composer::Intent::SetStatus(status) => model.global.status = Some(status),
            composer::Intent::ReplaceThreads(threads) => replace_threads(model, threads),
        }
    }
    result
        .effects
        .into_iter()
        .map(|effect| {
            let operation_id = model.global.set_pending(
                ActiveMode::Composer,
                effect::PendingEffectKind::ChangeThreads,
            );
            match effect {
                composer::Effect::ChangeThreads(operation) => Effect::ChangeThreads {
                    operation_id,
                    owner: ActiveMode::Composer,
                    operation,
                },
            }
        })
        .collect()
}

fn apply_help(model: &mut Model, result: help::Update) -> Vec<Effect> {
    for intent in result.intents {
        match intent {
            help::Intent::Close => model.active_mode = ActiveMode::Review,
            help::Intent::SetStatus(status) => model.global.status = Some(status.into()),
        }
    }
    result
        .effects
        .into_iter()
        .map(|effect| match effect {})
        .collect()
}

fn apply_rollup(model: &mut Model, result: rollup::Update) -> Vec<Effect> {
    for intent in result.intents {
        match intent {
            rollup::Intent::Close => model.active_mode = ActiveMode::Review,
            rollup::Intent::SetStatus(status) => model.global.status = Some(status),
            rollup::Intent::OpenThread { id, location } => {
                match model
                    .review
                    .select_thread_location(id, &location, &model.global.threads)
                {
                    Ok(filter_was_reset) => {
                        model.active_mode = ActiveMode::Review;
                        model.global.status = Some(if filter_was_reset {
                            format!(
                                "thread target: #{id}; filter reset to All changes to reveal target"
                            )
                        } else {
                            format!("thread target: #{id}")
                        });
                    }
                    Err(error) => model.global.status = Some(error),
                }
            }
        }
    }
    result
        .effects
        .into_iter()
        .map(|effect| {
            let operation_id = model
                .global
                .set_pending(ActiveMode::Rollup, effect::PendingEffectKind::ResolveThread);
            match effect {
                rollup::Effect::ResolveThread { id } => Effect::ResolveThread {
                    operation_id,
                    owner: ActiveMode::Rollup,
                    id,
                },
            }
        })
        .collect()
}

fn replace_threads(model: &mut Model, threads: ThreadState) {
    model
        .review
        .reconcile_replaced_threads(&model.global.threads, &threads);
    model.global.threads = threads;
}

pub fn view(model: &Model) -> semantic::View {
    let (file_rail, body, overlay, layout) = match model.active_mode {
        ActiveMode::Review => {
            let review = review_view(model);
            let overlay = thread_overlay(model);
            (review.file_rail, review.body, overlay, review.layout)
        }
        ActiveMode::Composer => {
            let review = review_view(model);
            (
                review.file_rail,
                review.body,
                Some(composer::view(
                    &model.composer,
                    model
                        .review
                        .selected_target_label(&model.global.threads)
                        .as_deref(),
                    model.global.keyboard_protocol,
                )),
                review.layout,
            )
        }
        ActiveMode::Help => {
            let review = review_view(model);
            (
                review.file_rail,
                review.body,
                Some(help::view(&model.help)),
                review.layout,
            )
        }
        ActiveMode::Rollup => {
            let review = review_view(model);
            let body = rollup::view(
                &model.rollup,
                rollup::ViewInput {
                    threads: &model.global.threads,
                },
            );
            (review.file_rail, body, None, review.layout)
        }
    };
    let selected_thread = model.review.selected_thread_id(&model.global.threads);
    let review_target = model.review.selected_target_label(&model.global.threads);
    let (context, target, selected_thread_available) = match model.active_mode {
        ActiveMode::Composer => (
            semantic::SurfaceContext::Composer,
            model.composer.reply_to().map_or_else(
                || {
                    review_target
                        .as_ref()
                        .map(|target| format!("comment on {target}"))
                },
                |id| Some(format!("reply to thread #{id}")),
            ),
            model.composer.reply_to().is_some(),
        ),
        ActiveMode::Help => (
            semantic::SurfaceContext::Help,
            Some("keyboard reference".into()),
            false,
        ),
        ActiveMode::Rollup => {
            let id = model.rollup.selected_thread_id(&model.global.threads);
            (
                semantic::SurfaceContext::Rollup,
                Some(id.map_or_else(|| "no threads".into(), |id| format!("thread #{id}"))),
                id.is_some(),
            )
        }
        ActiveMode::Review => match model.review.search_summary() {
            Some(search) if search.editing => (
                semantic::SurfaceContext::SearchInput,
                Some(if search.query.is_empty() {
                    "empty query • 0 matches".into()
                } else {
                    search.selected.map_or_else(
                        || format!("query “{}” • 0 matches", search.query),
                        |selected| {
                            format!(
                                "query “{}” • match {}/{}",
                                search.query,
                                selected + 1,
                                search.match_count
                            )
                        },
                    )
                }),
                false,
            ),
            Some(search) => (
                semantic::SurfaceContext::SearchResults,
                Some(search.selected.map_or_else(
                    || format!("“{}” • no matches", search.query),
                    |selected| {
                        format!(
                            "“{}” • match {}/{}",
                            search.query,
                            selected + 1,
                            search.match_count
                        )
                    },
                )),
                false,
            ),
            None => (
                match layout.focus {
                    crate::ui::FocusArea::Review => semantic::SurfaceContext::Review,
                    crate::ui::FocusArea::Threads => semantic::SurfaceContext::Threads,
                },
                match layout.focus {
                    crate::ui::FocusArea::Threads => selected_thread.map(|id| {
                        format!(
                            "{} • thread #{id}",
                            review_target.as_deref().unwrap_or("unknown hunk")
                        )
                    }),
                    crate::ui::FocusArea::Review => review_target,
                },
                selected_thread.is_some(),
            ),
        },
    };
    let selected_thread_resolved = selected_thread.is_some_and(|id| {
        model
            .global
            .threads
            .threads()
            .iter()
            .find(|thread| thread.id == id)
            .is_some_and(|thread| matches!(thread.resolution, Resolution::Resolved))
    });
    let global = global::view(
        &model.global,
        global::ViewInput {
            file_count: model.review.session().diff().document.files.len(),
            active_filter: model.review.filter().label(),
            context,
            target,
            selected_thread_available,
            selected_thread_resolved,
            width: model.review.viewport_columns(),
        },
    );
    semantic::View {
        header: global.header,
        file_rail,
        body,
        footer: global.footer,
        overlay,
        layout,
    }
}

fn review_view(model: &Model) -> review::View {
    review::view(
        &model.review,
        review::ViewInput {
            threads: &model.global.threads,
        },
    )
}

fn thread_overlay(model: &Model) -> Option<semantic::Overlay> {
    if model.review.focus() != crate::ui::FocusArea::Threads {
        return None;
    }
    let id = model.review.selected_thread_id(&model.global.threads)?;
    let thread = model.global.threads.thread(id)?;
    let (index, count) = model
        .review
        .selected_thread_position(&model.global.threads)?;
    let state = if thread.needs_attention {
        semantic::ThreadState::NeedsAttention
    } else if matches!(thread.resolution, Resolution::Open) {
        semantic::ThreadState::Open
    } else {
        semantic::ThreadState::Resolved
    };
    Some(semantic::Overlay::Thread(semantic::ThreadOverlay {
        id,
        context: model
            .review
            .selected_target_label(&model.global.threads)
            .unwrap_or_else(|| "Selected hunk".into()),
        state,
        outdated: thread.outdated,
        position: format!("{}/{}", index + 1, count),
        messages: thread
            .messages
            .iter()
            .map(|message| semantic::ThreadMessage {
                author: message.author.id.clone(),
                body: message.body.clone(),
            })
            .collect(),
    }))
}
