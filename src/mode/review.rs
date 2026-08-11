use crate::{
    anchor::HunkLocation,
    diff::{DiffRequest, LoadedDiff},
    input::{BindingResolution, Key, PhysicalInput},
    presentation::{self, ReviewRowMap, ViewportAnchor},
    review::ReviewSession,
    semantic::{
        Body, FileAttention, FileItem, FileRail, LayoutPolicy, ReviewBody, ReviewFile, ReviewHunk,
        ReviewViewport, ThreadCard, ThreadState as SemanticThreadState,
    },
    thread::{
        Resolution, ReviewThread, ThreadChange, ThreadId, ThreadOperation, ThreadState as Threads,
        ThreadSuccess,
    },
    ui::{FocusArea, LayoutMode, ViewState, review_body_width},
};

#[derive(Debug)]
pub struct Model {
    request: DiffRequest,
    session: ReviewSession,
    view: ViewState,
    sidebar_visible: bool,
    show_hunk_headers: bool,
    wrap_lines: bool,
    viewport_rows: u16,
    viewport_columns: u16,
}

impl Model {
    pub fn new(request: DiffRequest, diff: LoadedDiff) -> Self {
        Self {
            request,
            session: ReviewSession::new(diff),
            view: ViewState::default(),
            sidebar_visible: true,
            show_hunk_headers: true,
            wrap_lines: false,
            viewport_rows: 20,
            viewport_columns: 120,
        }
    }

    pub fn request(&self) -> &DiffRequest {
        &self.request
    }

    pub fn session(&self) -> &ReviewSession {
        &self.session
    }

    pub fn focus(&self) -> FocusArea {
        self.view.focus
    }

    #[cfg(test)]
    pub fn scroll(&self) -> usize {
        self.view.scroll
    }

    pub fn set_viewport(&mut self, rows: u16, columns: u16, threads: &Threads) {
        self.change_geometry(threads, |model| {
            model.viewport_rows = rows.max(1);
            model.viewport_columns = columns;
        });
    }

    pub fn viewport_columns(&self) -> u16 {
        self.viewport_columns
    }

    pub fn selected_location(&self) -> Option<HunkLocation> {
        self.session.selected_location()
    }

    pub fn select_thread_location(
        &mut self,
        id: ThreadId,
        location: &HunkLocation,
        threads: &Threads,
    ) -> Result<(), String> {
        let Some((file_index, file)) = self
            .session
            .diff()
            .document
            .files
            .iter()
            .enumerate()
            .find(|(_, file)| file.path == location.path())
        else {
            return Err(format!("thread #{id} anchor is not in this diff"));
        };
        let Some(hunk_index) = file
            .hunks
            .iter()
            .position(|hunk| hunk.header == location.hunk_header())
        else {
            return Err(format!("thread #{id} hunk is not in this diff"));
        };
        self.session.select_hunk(file_index, hunk_index);
        self.session.select_thread(
            threads
                .at(location)
                .iter()
                .position(|thread| thread.id == id)
                .unwrap_or(0),
        );
        self.view.focus = FocusArea::Threads;
        self.reveal_selected_target(threads);
        Ok(())
    }

    fn selected_threads<'a>(&self, threads: &'a Threads) -> Vec<&'a ReviewThread> {
        self.selected_location()
            .map(|location| threads.at(&location))
            .unwrap_or_default()
    }

    fn current_thread_id(&self, threads: &Threads) -> Option<ThreadId> {
        let selected = self.selected_threads(threads);
        selected
            .get(self.session.cursor().selected_thread())
            .or_else(|| selected.last())
            .map(|thread| thread.id)
    }

    pub fn selected_thread_id(&self, threads: &Threads) -> Option<ThreadId> {
        self.current_thread_id(threads)
    }

    pub fn selected_target_label(&self) -> Option<String> {
        let cursor = self.session.cursor();
        let file = self.session.file()?;
        file.hunks.get(cursor.selected_hunk())?;
        Some(format!(
            "{} • hunk {}/{}",
            file.path,
            cursor.selected_hunk() + 1,
            file.hunks.len()
        ))
    }

    fn selected_file_label(&self) -> Option<String> {
        let cursor = self.session.cursor();
        let files = &self.session.diff().document.files;
        let file = files.get(cursor.selected_file())?;
        Some(format!(
            "file {}/{}: {}",
            cursor.selected_file() + 1,
            files.len(),
            file.path
        ))
    }

    fn visible_rows(&self) -> usize {
        let sticky_rows = usize::from(!self.session.diff().document.files.is_empty());
        usize::from(self.viewport_rows)
            .saturating_sub(sticky_rows)
            .max(1)
    }

    fn row_map(&self, threads: &Threads) -> ReviewRowMap {
        let body = review_body(self, threads);
        presentation::review_row_map(
            &body,
            review_body_width(self.viewport_columns, self.sidebar_visible),
            LayoutPolicy {
                focus: self.focus(),
                diff_layout: self.view.layout,
                wrap_lines: self.wrap_lines,
            },
        )
    }

    fn viewport_anchor(&self, threads: &Threads) -> ViewportAnchor {
        let rows = self.row_map(threads);
        let top = self
            .view
            .resolved_scroll(rows.total_rows(), self.visible_rows());
        rows.anchor_at(top)
    }

    fn restore_viewport(&mut self, anchor: ViewportAnchor, threads: &Threads) {
        let rows = self.row_map(threads);
        let row = rows.row_for_anchor(&anchor);
        self.view.set_scroll(row);
        self.view.clamp(rows.total_rows(), self.visible_rows());
    }

    fn reveal_selected_target(&mut self, threads: &Threads) {
        let rows = self.row_map(threads);
        if let Some(row) = rows.selected_target_row() {
            self.view.reveal(row, self.visible_rows());
            self.view.clamp(rows.total_rows(), self.visible_rows());
        }
    }

    fn selected_target_is_visible(&self, threads: &Threads) -> bool {
        let rows = self.row_map(threads);
        let top = self
            .view
            .resolved_scroll(rows.total_rows(), self.visible_rows());
        rows.selected_target_row().is_some_and(|selected| {
            selected >= top && selected < top.saturating_add(self.visible_rows())
        })
    }

    fn change_geometry(&mut self, threads: &Threads, change: impl FnOnce(&mut Self)) {
        let selected_was_visible = self.selected_target_is_visible(threads);
        let preserve_end_relative = self.view.scroll_from_end.is_some();
        let anchor = self
            .view
            .scroll_from_end
            .is_none()
            .then(|| self.viewport_anchor(threads));
        change(self);
        if let Some(anchor) = anchor {
            self.restore_viewport(anchor, threads);
            if selected_was_visible {
                self.reveal_selected_target(threads);
            }
        } else if selected_was_visible && !self.selected_target_is_visible(threads) {
            self.reveal_selected_target(threads);
            if preserve_end_relative {
                let rows = self.row_map(threads);
                let max_scroll = rows.total_rows().saturating_sub(self.visible_rows());
                self.view.scroll_from_end = Some(max_scroll.saturating_sub(self.view.scroll));
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    CycleFocus,
    PreviousFocus,
    ScrollRows(i16),
    ScrollViewport(i16),
    ScrollHalfViewport(i16),
    JumpToStreamEdge { end: bool },
    MoveHunk(i32),
    MoveFile(i32),
    AdjustContext(i32),
    BeginThread { always_new: bool },
    MoveThread(i32),
    CloseThread,
    ReopenThread,
    ToggleAttention,
    ToggleOutdated,
    MoveAttention(i32),
    ShowRollup,
    SetLayout(LayoutMode),
    ToggleSidebar,
    ReloadDiff,
    ToggleHunkHeaders,
    ToggleWrap,
    EffectCompleted(Outcome),
}

impl Event {
    fn blocked_while_operation_pending(&self) -> bool {
        matches!(
            self,
            Self::AdjustContext(_)
                | Self::BeginThread { .. }
                | Self::CloseThread
                | Self::ReopenThread
                | Self::ToggleAttention
                | Self::ToggleOutdated
                | Self::MoveAttention(_)
                | Self::ReloadDiff
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReloadPurpose {
    ContextChanged,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    ReloadDiff {
        request: DiffRequest,
        purpose: ReloadPurpose,
    },
    ChangeThreads(ThreadOperation),
    ResolveThread {
        id: ThreadId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    DiffReloaded {
        purpose: ReloadPurpose,
        result: Result<LoadedDiff, String>,
    },
    ThreadsChanged {
        result: Result<ThreadChange, String>,
    },
    ThreadResolved {
        id: ThreadId,
        result: Result<HunkLocation, String>,
    },
}

pub struct UpdateInput<'a> {
    pub threads: &'a Threads,
    pub operation_pending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    SetStatus(String),
    ReplaceThreads(Threads),
    OpenComposer { reply_to: Option<ThreadId> },
    OpenRollup,
}

#[derive(Debug, Default)]
pub struct Update {
    pub intents: Vec<Intent>,
    pub effects: Vec<Effect>,
}

pub fn bindings(input: PhysicalInput) -> BindingResolution<Event> {
    if input.phase == crate::input::KeyPhase::Release {
        return BindingResolution::Consume;
    }
    if input.phase == crate::input::KeyPhase::Repeat
        && !matches!(
            input.key,
            Key::Char('j')
                | Key::Down
                | Key::Char('k')
                | Key::Up
                | Key::Char('f')
                | Key::PageDown
                | Key::Char('b')
                | Key::PageUp
                | Key::Char(' ')
                | Key::Char('d')
                | Key::Char('u')
                | Key::Char('g')
                | Key::Char('G')
                | Key::Home
                | Key::End
                | Key::Char(']')
                | Key::Char('[')
                | Key::Char('.')
                | Key::Char(',')
                | Key::Char('t')
                | Key::Char('T')
                | Key::Char('}')
                | Key::Char('{')
        )
    {
        return BindingResolution::Consume;
    }
    match input.key {
        Key::Char('q') | Key::Esc | Key::Char('?') => BindingResolution::Delegate,
        Key::Tab => BindingResolution::Handle(Event::CycleFocus),
        Key::BackTab => BindingResolution::Handle(Event::PreviousFocus),
        Key::Char('j') | Key::Down => BindingResolution::Handle(Event::ScrollRows(1)),
        Key::Char('k') | Key::Up => BindingResolution::Handle(Event::ScrollRows(-1)),
        Key::Char('f') | Key::PageDown => BindingResolution::Handle(Event::ScrollViewport(1)),
        Key::Char('b') | Key::PageUp => BindingResolution::Handle(Event::ScrollViewport(-1)),
        Key::Char(' ') if input.shift => BindingResolution::Handle(Event::ScrollViewport(-1)),
        Key::Char(' ') => BindingResolution::Handle(Event::ScrollViewport(1)),
        Key::Char('d') => BindingResolution::Handle(Event::ScrollHalfViewport(1)),
        Key::Char('u') => BindingResolution::Handle(Event::ScrollHalfViewport(-1)),
        Key::Char('g') | Key::Home => {
            BindingResolution::Handle(Event::JumpToStreamEdge { end: false })
        }
        Key::Char('G') | Key::End => {
            BindingResolution::Handle(Event::JumpToStreamEdge { end: true })
        }
        Key::Char(']') => BindingResolution::Handle(Event::MoveHunk(1)),
        Key::Char('[') => BindingResolution::Handle(Event::MoveHunk(-1)),
        Key::Char('.') => BindingResolution::Handle(Event::MoveFile(1)),
        Key::Char(',') => BindingResolution::Handle(Event::MoveFile(-1)),
        Key::Char('=') => BindingResolution::Handle(Event::AdjustContext(1)),
        Key::Char('-') => BindingResolution::Handle(Event::AdjustContext(-1)),
        Key::Char('c') => BindingResolution::Handle(Event::BeginThread { always_new: false }),
        Key::Char('C') => BindingResolution::Handle(Event::BeginThread { always_new: true }),
        Key::Char('t') => BindingResolution::Handle(Event::MoveThread(1)),
        Key::Char('T') => BindingResolution::Handle(Event::MoveThread(-1)),
        Key::Char('x') => BindingResolution::Handle(Event::CloseThread),
        Key::Char('R') => BindingResolution::Handle(Event::ReopenThread),
        Key::Char('a') => BindingResolution::Handle(Event::ToggleAttention),
        Key::Char('o') => BindingResolution::Handle(Event::ToggleOutdated),
        Key::Char('}') => BindingResolution::Handle(Event::MoveAttention(1)),
        Key::Char('{') => BindingResolution::Handle(Event::MoveAttention(-1)),
        Key::Char('v') => BindingResolution::Handle(Event::ShowRollup),
        Key::Char('1') => BindingResolution::Handle(Event::SetLayout(LayoutMode::Split)),
        Key::Char('2') => BindingResolution::Handle(Event::SetLayout(LayoutMode::Stack)),
        Key::Char('0') => BindingResolution::Handle(Event::SetLayout(LayoutMode::Auto)),
        Key::Char('s') => BindingResolution::Handle(Event::ToggleSidebar),
        Key::Char('r') => BindingResolution::Handle(Event::ReloadDiff),
        Key::Char('m') => BindingResolution::Handle(Event::ToggleHunkHeaders),
        Key::Char('w') => BindingResolution::Handle(Event::ToggleWrap),
        _ => BindingResolution::Unbound,
    }
}

pub fn update(model: &mut Model, event: Event, input: UpdateInput<'_>) -> Update {
    let mut result = Update::default();
    if input.operation_pending && event.blocked_while_operation_pending() {
        status(
            &mut result,
            "cannot start another action while an operation is pending",
        );
        return result;
    }
    match event {
        Event::CycleFocus => {
            move_focus(model, input.threads, &mut result, false);
        }
        Event::PreviousFocus => move_focus(model, input.threads, &mut result, true),
        Event::ScrollRows(delta) => {
            model.view.scroll_by(delta);
            let rows = model.row_map(input.threads);
            model.view.clamp(rows.total_rows(), model.visible_rows());
            status(&mut result, scroll_status(&model.view));
        }
        Event::ScrollViewport(direction) => {
            let viewport = model.visible_rows().min(i16::MAX as usize) as i16;
            model.view.scroll_by(direction.saturating_mul(viewport));
            let rows = model.row_map(input.threads);
            model.view.clamp(rows.total_rows(), model.visible_rows());
            status(&mut result, scroll_status(&model.view));
        }
        Event::ScrollHalfViewport(direction) => {
            let half_viewport = (model.visible_rows() / 2).max(1).min(i16::MAX as usize) as i16;
            model
                .view
                .scroll_by(direction.saturating_mul(half_viewport));
            let rows = model.row_map(input.threads);
            model.view.clamp(rows.total_rows(), model.visible_rows());
            status(&mut result, scroll_status(&model.view));
        }
        Event::JumpToStreamEdge { end } => {
            if end {
                model.view.jump_to_end();
            } else {
                model.view.set_scroll(0);
            }
            status(
                &mut result,
                if end {
                    "review stream end"
                } else {
                    "review stream start"
                },
            );
        }
        Event::MoveHunk(direction) => {
            if model.session.move_hunk(direction) {
                model.view.focus = FocusArea::Review;
                model.reveal_selected_target(input.threads);
                status(
                    &mut result,
                    format!(
                        "hunk target: {}",
                        model
                            .selected_target_label()
                            .unwrap_or_else(|| "unavailable".into())
                    ),
                );
            } else {
                status(&mut result, "cannot move hunks: this diff has no hunks");
            }
        }
        Event::MoveFile(direction) => {
            if model.session.move_file(direction) {
                model.view.focus = FocusArea::Review;
                model.reveal_selected_target(input.threads);
                status(
                    &mut result,
                    format!(
                        "file target: {}",
                        model
                            .selected_file_label()
                            .unwrap_or_else(|| "unavailable".into())
                    ),
                );
            } else {
                status(
                    &mut result,
                    "cannot move files: this diff has no changed files",
                );
            }
        }
        Event::AdjustContext(delta) => {
            let context = model.request.context_lines as i32 + delta;
            if context >= 0 {
                model.request.context_lines = context as usize;
                result.effects.push(Effect::ReloadDiff {
                    request: model.request.clone(),
                    purpose: ReloadPurpose::ContextChanged,
                });
            } else {
                status(&mut result, "context already has 0 lines");
            }
        }
        Event::BeginThread { always_new } => {
            if model.selected_location().is_none() {
                status(&mut result, "select a hunk before posting a thread");
            } else {
                let reply_to = if !always_new && model.focus() == FocusArea::Threads {
                    model.current_thread_id(input.threads)
                } else {
                    None
                };
                status(
                    &mut result,
                    reply_to.map_or_else(
                        || "composing a new thread".into(),
                        |id| format!("replying to thread #{id}"),
                    ),
                );
                result.intents.push(Intent::OpenComposer { reply_to });
            }
        }
        Event::MoveThread(direction) => {
            let count = model.selected_threads(input.threads).len();
            if count == 0 {
                status(
                    &mut result,
                    "cannot select a thread: this hunk has no threads",
                );
            } else {
                let delta = if model.view.focus == FocusArea::Threads {
                    direction
                } else {
                    0
                };
                let selected =
                    wrapped_index(model.session.cursor().selected_thread(), count, delta);
                model.session.select_thread(selected);
                model.view.focus = FocusArea::Threads;
                model.reveal_selected_target(input.threads);
                let id = model
                    .current_thread_id(input.threads)
                    .expect("thread count is non-zero");
                status(
                    &mut result,
                    format!("thread target: #{id} ({}/{count})", selected + 1),
                );
            }
        }
        Event::CloseThread => {
            thread_effect(
                model,
                input.threads,
                &mut result,
                "could not close thread",
                |id| ThreadOperation::Close { id },
            );
        }
        Event::ReopenThread => {
            thread_effect(
                model,
                input.threads,
                &mut result,
                "could not reopen thread",
                |id| ThreadOperation::Reopen { id },
            );
        }
        Event::ToggleAttention => {
            let Some(id) = current_thread(model, input.threads, &mut result) else {
                return result;
            };
            let value = !input
                .threads
                .thread(id)
                .is_some_and(|thread| thread.needs_attention);
            result
                .effects
                .push(Effect::ChangeThreads(ThreadOperation::SetAttention {
                    id,
                    value,
                }));
        }
        Event::ToggleOutdated => {
            let Some(id) = current_thread(model, input.threads, &mut result) else {
                return result;
            };
            let value = !input
                .threads
                .thread(id)
                .is_some_and(|thread| thread.outdated);
            result
                .effects
                .push(Effect::ChangeThreads(ThreadOperation::SetOutdated {
                    id,
                    value,
                }));
        }
        Event::MoveAttention(direction) => {
            let attention = input.threads.attention_ids();
            if attention.is_empty() {
                status(&mut result, "no needs-attention threads");
            } else {
                let current = model.current_thread_id(input.threads);
                let index = current
                    .and_then(|id| attention.iter().position(|candidate| *candidate == id))
                    .unwrap_or(0);
                let id = attention[wrapped_index(index, attention.len(), direction)];
                result.effects.push(Effect::ResolveThread { id });
            }
        }
        Event::ShowRollup => {
            status(&mut result, "opened thread rollup");
            result.intents.push(Intent::OpenRollup);
        }
        Event::SetLayout(layout) => {
            model.change_geometry(input.threads, |model| model.view.layout = layout);
            status(&mut result, format!("layout: {}", layout_label(layout)));
        }
        Event::ToggleSidebar => {
            model.change_geometry(input.threads, |model| {
                model.sidebar_visible = !model.sidebar_visible;
            });
            let message = if !model.sidebar_visible {
                "file rail hidden"
            } else if model.viewport_columns < 72 {
                "file rail enabled; hidden below 72 columns"
            } else {
                "file rail shown"
            };
            status(&mut result, message);
        }
        Event::ReloadDiff => result.effects.push(Effect::ReloadDiff {
            request: model.request.clone(),
            purpose: ReloadPurpose::Manual,
        }),
        Event::ToggleHunkHeaders => {
            model.change_geometry(input.threads, |model| {
                model.show_hunk_headers = !model.show_hunk_headers;
            });
            status(
                &mut result,
                if model.show_hunk_headers {
                    "hunk headers shown"
                } else {
                    "hunk headers hidden"
                },
            );
        }
        Event::ToggleWrap => {
            model.change_geometry(input.threads, |model| {
                model.wrap_lines = !model.wrap_lines;
            });
            status(
                &mut result,
                if model.wrap_lines {
                    "line wrapping enabled"
                } else {
                    "line wrapping disabled"
                },
            );
        }
        Event::EffectCompleted(outcome) => {
            apply_outcome(model, input.threads, outcome, &mut result)
        }
    }
    result
}

fn move_focus(model: &mut Model, threads: &Threads, result: &mut Update, previous: bool) {
    let next = if previous {
        model.view.focus.previous()
    } else {
        model.view.focus.next()
    };
    if next == FocusArea::Threads && model.selected_threads(threads).is_empty() {
        status(result, "cannot focus threads: this hunk has no threads");
        return;
    }
    model.view.focus = next;
    match next {
        FocusArea::Review => status(result, "review stream focused"),
        FocusArea::Threads => {
            model.reveal_selected_target(threads);
            let id = model
                .current_thread_id(threads)
                .expect("thread focus requires a selected thread");
            status(result, format!("thread target: #{id}"));
        }
    }
}

fn scroll_status(view: &ViewState) -> String {
    match view.scroll_from_end {
        Some(0) => "review stream end".into(),
        Some(1) => "review stream 1 row before end".into(),
        Some(offset) => format!("review stream {offset} rows before end"),
        None => format!("review stream row {}", view.scroll + 1),
    }
}

fn layout_label(layout: LayoutMode) -> &'static str {
    match layout {
        LayoutMode::Auto => "responsive",
        LayoutMode::Split => "split",
        LayoutMode::Stack => "stack",
    }
}

fn apply_outcome(model: &mut Model, threads: &Threads, outcome: Outcome, result: &mut Update) {
    match outcome {
        Outcome::DiffReloaded {
            purpose,
            result: outcome,
        } => match outcome {
            Ok(diff) => {
                let preserve_end = model.view.scroll_from_end.is_some();
                let viewport_anchor = (!preserve_end).then(|| model.viewport_anchor(threads));
                let selected_was_visible = model.selected_target_is_visible(threads);
                let previous_thread_target = (model.focus() == FocusArea::Threads)
                    .then(|| {
                        Some((
                            model.selected_location()?,
                            model.current_thread_id(threads)?,
                        ))
                    })
                    .flatten();
                model.session.replace_diff(diff);
                let restored_thread_target =
                    previous_thread_target.is_some_and(|(location, id)| {
                        if model.selected_location().as_ref() != Some(&location) {
                            return false;
                        }
                        let Some(index) = model
                            .selected_threads(threads)
                            .iter()
                            .position(|thread| thread.id == id)
                        else {
                            return false;
                        };
                        model.session.select_thread(index);
                        true
                    });
                if model.focus() == FocusArea::Threads && !restored_thread_target {
                    model.view.focus = FocusArea::Review;
                }
                if let Some(viewport_anchor) = viewport_anchor {
                    model.restore_viewport(viewport_anchor, threads);
                    if selected_was_visible {
                        model.reveal_selected_target(threads);
                    }
                }
                let message = match purpose {
                    ReloadPurpose::ContextChanged => {
                        format!("context: {} lines", model.request.context_lines)
                    }
                    ReloadPurpose::Manual => "reloaded current diff".into(),
                };
                status(result, message);
            }
            Err(error) => status(result, format!("could not reload diff: {error}")),
        },
        Outcome::ThreadsChanged { result: outcome } => match outcome {
            Ok(change) => {
                status(result, success_status(change.success));
                result.intents.push(Intent::ReplaceThreads(change.state));
            }
            Err(error) => status(result, error),
        },
        Outcome::ThreadResolved {
            id,
            result: outcome,
        } => match outcome {
            Ok(location) => match model.select_thread_location(id, &location, threads) {
                Ok(()) => status(result, format!("thread #{id}")),
                Err(error) => status(result, error),
            },
            Err(error) => status(
                result,
                format!("thread #{id} anchor cannot resolve: {error}"),
            ),
        },
    }
}

fn current_thread(model: &Model, threads: &Threads, result: &mut Update) -> Option<ThreadId> {
    if model.focus() != FocusArea::Threads {
        status(
            result,
            "could not update thread: select a thread with t first",
        );
        return None;
    }
    let id = model.current_thread_id(threads);
    if id.is_none() {
        status(result, "could not update thread: no thread on this hunk");
    }
    id
}

fn thread_effect(
    model: &Model,
    threads: &Threads,
    result: &mut Update,
    error_prefix: &str,
    operation: impl FnOnce(ThreadId) -> ThreadOperation,
) {
    if model.focus() != FocusArea::Threads {
        status(
            result,
            format!("{error_prefix}: select a thread with t first"),
        );
        return;
    }
    let Some(id) = model.current_thread_id(threads) else {
        status(result, format!("{error_prefix}: no thread on this hunk"));
        return;
    };
    result.effects.push(Effect::ChangeThreads(operation(id)));
}

fn status(result: &mut Update, message: impl Into<String>) {
    result.intents.push(Intent::SetStatus(message.into()));
}

fn success_status(success: ThreadSuccess) -> String {
    match success {
        ThreadSuccess::Posted(id) => format!("posted thread #{id}"),
        ThreadSuccess::Replied => "posted reply".into(),
        ThreadSuccess::Closed => "thread closed".into(),
        ThreadSuccess::Reopened => "thread reopened".into(),
        ThreadSuccess::AttentionToggled => "needs-attention toggled".into(),
        ThreadSuccess::OutdatedToggled => "outdated toggled".into(),
    }
}

pub struct ViewInput<'a> {
    pub threads: &'a Threads,
}

pub struct View {
    pub body: Body,
    pub file_rail: Option<FileRail>,
    pub layout: LayoutPolicy,
}

pub fn view(model: &Model, input: ViewInput<'_>) -> View {
    let file_rail = model.sidebar_visible.then(|| FileRail {
        selected: (!model.session.diff().document.files.is_empty())
            .then(|| model.session.cursor().selected_file()),
        items: model
            .session
            .diff()
            .document
            .files
            .iter()
            .map(|file| {
                let threads = input.threads.in_file(&file.path);
                let attention = if threads.iter().any(|thread| thread.needs_attention) {
                    FileAttention::NeedsAttention
                } else if threads
                    .iter()
                    .any(|thread| matches!(thread.resolution, Resolution::Open))
                {
                    FileAttention::Open
                } else if threads.is_empty() {
                    FileAttention::None
                } else {
                    FileAttention::Resolved
                };
                FileItem {
                    path: file.path.clone(),
                    hunk_count: file.hunks.len(),
                    thread_count: threads.len(),
                    attention,
                }
            })
            .collect(),
    });
    let mut review = review_body(model, input.threads);
    let presentation_width = review_body_width(model.viewport_columns, model.sidebar_visible);
    let rows = presentation::review_row_map(
        &review,
        presentation_width,
        LayoutPolicy {
            focus: model.focus(),
            diff_layout: model.view.layout,
            wrap_lines: model.wrap_lines,
        },
    );
    let scroll = model
        .view
        .resolved_scroll(rows.total_rows(), model.visible_rows());
    review.scroll = scroll;
    review.viewport = ReviewViewport {
        presentation_width,
        total_rows: rows.total_rows(),
        visible_rows: model.visible_rows(),
        sticky_context: rows.sticky_context(scroll),
    };
    let body = Body::Review(review);
    View {
        body,
        file_rail,
        layout: LayoutPolicy {
            focus: model.focus(),
            diff_layout: model.view.layout,
            wrap_lines: model.wrap_lines,
        },
    }
}

fn review_body(model: &Model, threads: &Threads) -> ReviewBody {
    ReviewBody {
        scroll: 0,
        viewport: ReviewViewport {
            presentation_width: 0,
            total_rows: 0,
            visible_rows: 0,
            sticky_context: None,
        },
        empty_state: model
            .session
            .diff()
            .document
            .files
            .is_empty()
            .then(|| empty_diff_message(&model.request.target)),
        files: model
            .session
            .diff()
            .document
            .files
            .iter()
            .enumerate()
            .map(|(file_index, file)| ReviewFile {
                path: file.path.clone(),
                selected: file_index == model.session.cursor().selected_file(),
                extension: file.extension().map(str::to_owned),
                metadata: file.metadata.clone(),
                hunks: file
                    .hunks
                    .iter()
                    .enumerate()
                    .map(|(hunk_index, hunk)| {
                        let selected = file_index == model.session.cursor().selected_file()
                            && hunk_index == model.session.cursor().selected_hunk();
                        let location = HunkLocation::new(&file.path, &hunk.header);
                        ReviewHunk {
                            anchor: location.clone(),
                            header: model.show_hunk_headers.then(|| hunk.header.clone()),
                            coordinates: hunk.coordinates,
                            selected,
                            lines: hunk.lines.clone(),
                            threads: threads
                                .at(&location)
                                .iter()
                                .enumerate()
                                .map(|(thread_index, thread)| ThreadCard {
                                    id: thread.id,
                                    state: thread_state(thread),
                                    outdated: thread.outdated,
                                    active: selected
                                        && thread_index == model.session.cursor().selected_thread()
                                        && model.focus() == FocusArea::Threads,
                                    latest: thread
                                        .messages
                                        .last()
                                        .map(|message| {
                                            format!("{}: {}", message.author.id, message.body)
                                        })
                                        .unwrap_or_default(),
                                })
                                .collect(),
                        }
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn empty_diff_message(target: &crate::diff::DiffTarget) -> String {
    format!(
        "No changes found.\nSelected diff target: {}.\nUpdate the target or make a change, then press r to reload.",
        target.description()
    )
}

fn thread_state(thread: &ReviewThread) -> SemanticThreadState {
    if thread.needs_attention {
        SemanticThreadState::NeedsAttention
    } else if matches!(thread.resolution, Resolution::Open) {
        SemanticThreadState::Open
    } else {
        SemanticThreadState::Resolved
    }
}

fn wrapped_index(current: usize, length: usize, direction: i32) -> usize {
    ((current as i32 + direction).rem_euclid(length as i32)) as usize
}
