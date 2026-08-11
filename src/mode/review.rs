use crate::{
    anchor::HunkLocation,
    diff::{DiffRequest, LoadedDiff},
    input::{BindingResolution, Key, PhysicalInput},
    review::ReviewSession,
    semantic::{
        Body, FileAttention, FileItem, FileRail, LayoutPolicy, ReviewBody, ReviewFile, ReviewHunk,
        ThreadCard, ThreadState as SemanticThreadState,
    },
    thread::{
        Resolution, ReviewThread, ThreadChange, ThreadId, ThreadOperation, ThreadState as Threads,
        ThreadSuccess,
    },
    ui::{FocusArea, LayoutMode, ViewState},
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

    pub fn scroll(&self) -> u16 {
        self.view.scroll
    }

    pub fn set_viewport_rows(&mut self, rows: u16) {
        self.viewport_rows = rows.max(1);
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
        self.view.scroll = self.hunk_start_line(threads);
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

    fn hunk_start_line(&self, threads: &Threads) -> u16 {
        let mut lines = 0usize;
        for (file_index, file) in self.session.diff().document.files.iter().enumerate() {
            lines += 2 + file.metadata.len();
            for (hunk_index, hunk) in file.hunks.iter().enumerate() {
                if file_index == self.session.cursor().selected_file()
                    && hunk_index == self.session.cursor().selected_hunk()
                {
                    return lines.try_into().unwrap_or(u16::MAX);
                }
                let location = HunkLocation::new(&file.path, &hunk.header);
                lines += 1 + hunk.lines.len() + threads.at(&location).len() * 3;
            }
        }
        0
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
    FocusReview,
    MoveHunk(i32),
    MoveFile(i32),
    AdjustContext(i32),
    BeginThread { always_new: bool },
    SelectThread,
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
        Key::Enter => BindingResolution::Handle(Event::FocusReview),
        Key::Char(']') => BindingResolution::Handle(Event::MoveHunk(1)),
        Key::Char('[') => BindingResolution::Handle(Event::MoveHunk(-1)),
        Key::Char('.') => BindingResolution::Handle(Event::MoveFile(1)),
        Key::Char(',') => BindingResolution::Handle(Event::MoveFile(-1)),
        Key::Char('=') => BindingResolution::Handle(Event::AdjustContext(1)),
        Key::Char('-') => BindingResolution::Handle(Event::AdjustContext(-1)),
        Key::Char('c') => BindingResolution::Handle(Event::BeginThread { always_new: false }),
        Key::Char('C') => BindingResolution::Handle(Event::BeginThread { always_new: true }),
        Key::Char('t') => BindingResolution::Handle(Event::SelectThread),
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
    match event {
        Event::CycleFocus => {
            model.view.focus = model.view.focus.next();
            if model.view.focus == FocusArea::Threads
                && model.selected_threads(input.threads).is_empty()
            {
                model.view.focus = FocusArea::Files;
                status(
                    &mut result,
                    "this hunk has no threads; focus moved to files",
                );
            }
        }
        Event::PreviousFocus => model.view.focus = model.view.focus.previous(),
        Event::ScrollRows(delta) => model.view.scroll_by(delta),
        Event::ScrollViewport(direction) => model
            .view
            .scroll_by(direction.saturating_mul(model.viewport_rows as i16)),
        Event::ScrollHalfViewport(direction) => model
            .view
            .scroll_by(direction.saturating_mul((model.viewport_rows / 2).max(1) as i16)),
        Event::JumpToStreamEdge { end } => {
            model.view.scroll = if end { u16::MAX } else { 0 };
        }
        Event::FocusReview if model.view.focus == FocusArea::Files => {
            model.view.focus = FocusArea::Review;
            model.view.scroll = model.hunk_start_line(input.threads);
        }
        Event::FocusReview => {}
        Event::MoveHunk(direction) => {
            if model.session.move_hunk(direction) {
                let row = model.hunk_start_line(input.threads);
                model.view.reveal(row, model.viewport_rows);
            }
        }
        Event::MoveFile(direction) => {
            if model.session.move_file(direction) {
                model.view.scroll = model.hunk_start_line(input.threads);
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
                result.intents.push(Intent::OpenComposer { reply_to });
            }
        }
        Event::SelectThread => {
            let count = model.selected_threads(input.threads).len();
            if count == 0 {
                status(&mut result, "this hunk has no threads");
            } else {
                let selected = wrapped_index(model.session.cursor().selected_thread(), count, 0);
                model.session.select_thread(selected);
                model.view.focus = FocusArea::Threads;
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
        Event::ShowRollup => result.intents.push(Intent::OpenRollup),
        Event::SetLayout(layout) => model.view.layout = layout,
        Event::ToggleSidebar => model.sidebar_visible = !model.sidebar_visible,
        Event::ReloadDiff => result.effects.push(Effect::ReloadDiff {
            request: model.request.clone(),
            purpose: ReloadPurpose::Manual,
        }),
        Event::ToggleHunkHeaders => model.show_hunk_headers = !model.show_hunk_headers,
        Event::ToggleWrap => model.wrap_lines = !model.wrap_lines,
        Event::EffectCompleted(outcome) => {
            apply_outcome(model, input.threads, outcome, &mut result)
        }
    }
    result
}

fn apply_outcome(model: &mut Model, threads: &Threads, outcome: Outcome, result: &mut Update) {
    match outcome {
        Outcome::DiffReloaded {
            purpose,
            result: outcome,
        } => match outcome {
            Ok(diff) => {
                model.session.replace_diff(diff);
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
        focused: model.focus() == FocusArea::Files,
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
    let body = Body::Review(ReviewBody {
        focused: model.focus() == FocusArea::Review,
        scroll: model.scroll(),
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
                            header: model.show_hunk_headers.then(|| hunk.header.clone()),
                            coordinates: hunk.coordinates,
                            selected,
                            lines: hunk.lines.clone(),
                            threads: input
                                .threads
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
    });
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
