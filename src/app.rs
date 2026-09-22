// Tree and table presentation portions are adapted from xpdig's
// internal/bubbles/layout/xpnavigator/model.go.
// Copyright 2025 Bruno Luiz da Silva. Licensed under Apache-2.0.
// Translated and substantially modified for xpdelve in 2026. See NOTICE.
use std::collections::{HashSet, VecDeque};
use std::io::{self, IsTerminal, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event as TerminalEvent, EventStream, KeyCode,
    KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures_util::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use regex::RegexBuilder;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use tokio::time;
use tokio_util::sync::CancellationToken;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::cli::Cli;
use crate::config::Config;
use crate::kubernetes::{DeletePropagation, Kubernetes, Target};
use crate::model::{Identity, ProjectedNode, Snapshot};
use crate::text;
use crate::theme::Theme;
use crate::trace::{self, TraceRequest};

const EVENT_BUFFER: usize = 128;
const DESCRIBE_OUTPUT_LIMIT: usize = 16 * 1024 * 1024;
const DESCRIBE_ERROR_LIMIT: usize = 1024 * 1024;
const DESCRIBE_TIMEOUT: Duration = Duration::from_secs(60);
const TOAST_DURATION: Duration = Duration::from_millis(1500);
const HELP_LINES: &[&str] = &[
    "Navigation",
    "  j/k, Up/Down      move selection",
    "  PgUp/PgDn         move one page",
    "  Enter/Space       toggle expand or collapse",
    "  Right             expand or select first child",
    "  Left              collapse or select parent",
    "  [                 collapse all",
    "  ]                 expand all",
    "  z                 toggle fitted / full-width table",
    "  Alt+h / Alt+l     scroll full-width table",
    "",
    "Discovery",
    "  /                 filter tree",
    "  f                 find text",
    "  n / N             next / previous match",
    "  Esc               clear find and filter",
    "",
    "Session",
    "  r                 refresh now",
    "  P                 pause automatic refresh",
    "  q / Ctrl+C        quit",
    "",
    "Actions",
    "  d / y / v         describe / live YAML / events",
    "  e                 kubectl edit",
    "  c                 copy resource identifier",
    "  p / u             pause / unpause resource",
    "  Ctrl+D            delete resource",
    "  Ctrl+X            remove all finalizers",
];

enum AppEvent {
    TraceFinished {
        generation: u64,
        result: TraceResult,
    },
    KubernetesReady(Result<Arc<Kubernetes>, String>),
    ActionFinished {
        label: String,
        identity: Option<Identity>,
        result: Result<Option<String>, String>,
        refresh: bool,
    },
    Retry {
        generation: u64,
    },
}

enum TraceResult {
    Snapshot(Snapshot),
    ResourceNotFound,
    Failed(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InputMode {
    Normal,
    Filter,
    Find,
    Help,
}

#[derive(Clone, Debug)]
enum Modal {
    Text {
        title: String,
        content: String,
        kind: ContentKind,
        wrapped: bool,
        vertical_scroll: u16,
        horizontal_scroll: u16,
        query: String,
        search_input: Option<String>,
        selection: Option<TextSelection>,
    },
    Delete {
        target: Target,
        propagation: DeletePropagation,
    },
    Finalizers {
        target: Target,
        finalizers: Vec<String>,
        selected: HashSet<usize>,
        cursor: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SelectionPoint {
    start: usize,
    end: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TextSelection {
    anchor: SelectionPoint,
    focus: SelectionPoint,
}

impl TextSelection {
    fn range(self) -> std::ops::Range<usize> {
        self.anchor.start.min(self.focus.start)..self.anchor.end.max(self.focus.end)
    }
}

#[derive(Clone, Debug)]
struct Toast {
    message: String,
    expires_at: Instant,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContentKind {
    Describe,
    Yaml,
    Events,
}

impl ContentKind {
    fn supports_mouse_selection(self) -> bool {
        matches!(self, Self::Describe | Self::Yaml | Self::Events)
    }
}

struct App {
    resource: String,
    snapshot: Option<Arc<Snapshot>>,
    selected_identity: Option<Identity>,
    selected_visible: usize,
    resource_scroll: usize,
    collapsed: HashSet<Identity>,
    mode: InputMode,
    help_scroll: u16,
    input: String,
    filter: String,
    find: String,
    loading: bool,
    paused: bool,
    generation: u64,
    last_refresh: Option<Instant>,
    status: String,
    quit: bool,
    config: Config,
    no_watch: bool,
    theme: Theme,
    kubernetes: Option<Arc<Kubernetes>>,
    kubernetes_status: String,
    modal: Option<Modal>,
    active_mutations: HashSet<Identity>,
    refresh_pending: bool,
    retry_delay: Duration,
    full_width: bool,
    horizontal_offset: u16,
    resource_missing: bool,
    namespace: Option<String>,
    context: Option<String>,
    toast: Option<Toast>,
}

impl App {
    fn new(resource: String, config: Config, cli: &Cli, theme: Theme) -> Self {
        let full_width = config.ui.horizontal_scroll;
        Self {
            resource,
            snapshot: None,
            selected_identity: None,
            selected_visible: 0,
            resource_scroll: 0,
            collapsed: HashSet::new(),
            mode: InputMode::Normal,
            help_scroll: 0,
            input: String::new(),
            filter: String::new(),
            find: String::new(),
            loading: false,
            paused: false,
            generation: 0,
            last_refresh: None,
            status: "Starting trace...".into(),
            quit: false,
            config,
            no_watch: cli.no_watch,
            theme,
            kubernetes: None,
            kubernetes_status: "Kubernetes client initializing".into(),
            modal: None,
            active_mutations: HashSet::new(),
            refresh_pending: false,
            retry_delay: Duration::from_secs(1),
            full_width,
            horizontal_offset: 0,
            resource_missing: false,
            namespace: cli.namespace.clone(),
            context: cli.context.clone(),
            toast: None,
        }
    }

    fn show_toast(&mut self, message: impl Into<String>) {
        self.toast = Some(Toast {
            message: message.into(),
            expires_at: Instant::now() + TOAST_DURATION,
        });
    }

    fn visible(&self) -> Vec<usize> {
        self.snapshot.as_ref().map_or_else(Vec::new, |snapshot| {
            snapshot.visible_indices(
                &self.collapsed,
                (!self.filter.is_empty()).then_some(self.filter.as_str()),
            )
        })
    }

    fn selected_node(&self) -> Option<&ProjectedNode> {
        let visible = self.visible();
        let index = *visible.get(self.selected_visible)?;
        self.snapshot.as_ref()?.nodes.get(index)
    }

    fn selected_target(&self) -> Option<Target> {
        self.selected_node().map(|node| Target {
            identity: node.identity.clone(),
            expected_uid: node.uid.clone(),
        })
    }

    fn apply_snapshot(&mut self, snapshot: Snapshot) {
        let selection_chain = self.selection_chain();
        let snapshot = Arc::new(snapshot);
        self.snapshot = Some(Arc::clone(&snapshot));
        self.collapsed
            .retain(|identity| snapshot.by_identity.contains_key(identity));
        let visible = self.visible();
        self.selected_visible = selection_chain
            .iter()
            .find_map(|identity| snapshot.by_identity.get(identity))
            .and_then(|index| {
                visible
                    .iter()
                    .position(|visible_index| visible_index == index)
            })
            .unwrap_or_else(|| self.selected_visible.min(visible.len().saturating_sub(1)));
        self.sync_selected_identity();
        self.loading = false;
        self.resource_missing = false;
        self.last_refresh = Some(Instant::now());
        self.status = format!("Trace updated: {} resources", snapshot.nodes.len());
        self.retry_delay = Duration::from_secs(1);
    }

    fn apply_resource_not_found(&mut self) {
        self.snapshot = None;
        self.selected_identity = None;
        self.selected_visible = 0;
        self.resource_scroll = 0;
        self.collapsed.clear();
        self.modal = None;
        self.refresh_pending = false;
        self.loading = false;
        self.resource_missing = true;
        self.status.clear();
        self.retry_delay = Duration::from_secs(1);
    }

    fn selection_chain(&self) -> Vec<Identity> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        let visible = self.visible();
        let Some(mut index) = visible.get(self.selected_visible).copied() else {
            return Vec::new();
        };
        let mut identities = Vec::new();
        loop {
            let node = &snapshot.nodes[index];
            identities.push(node.identity.clone());
            let Some(parent) = node.parent else {
                break;
            };
            index = parent;
        }
        identities
    }

    fn sync_selected_identity(&mut self) {
        self.selected_identity = self.selected_node().map(|node| node.identity.clone());
    }

    fn move_selection(&mut self, delta: isize) {
        let len = self.visible().len();
        if len == 0 {
            self.selected_visible = 0;
            self.selected_identity = None;
            return;
        }
        self.selected_visible = self
            .selected_visible
            .saturating_add_signed(delta)
            .min(len.saturating_sub(1));
        self.sync_selected_identity();
    }

    fn set_selection(&mut self, index: usize) {
        self.selected_visible = index.min(self.visible().len().saturating_sub(1));
        self.sync_selected_identity();
    }

    fn ensure_selection_visible(&mut self, viewport: usize) {
        self.resource_scroll = self.resource_view_start(viewport);
    }

    fn resource_view_start(&self, viewport: usize) -> usize {
        if viewport == 0 {
            return 0;
        }
        let len = self.visible().len();
        let mut start = self.resource_scroll.min(len.saturating_sub(viewport));
        if self.selected_visible < start {
            start = self.selected_visible;
        } else if self.selected_visible >= start.saturating_add(viewport) {
            start = self
                .selected_visible
                .saturating_sub(viewport.saturating_sub(1));
        }
        start
    }

    fn toggle_selected(&mut self) {
        let Some(node) = self.selected_node() else {
            return;
        };
        if node.child_count == 0 {
            return;
        }
        let identity = node.identity.clone();
        if !self.collapsed.remove(&identity) {
            self.collapsed.insert(identity);
        }
    }

    fn expand_or_child(&mut self) {
        let visible = self.visible();
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let Some(index) = visible.get(self.selected_visible).copied() else {
            return;
        };
        let node = &snapshot.nodes[index];
        if node.child_count == 0 {
            return;
        }
        if self.collapsed.remove(&node.identity) {
            return;
        }
        if let Some(position) = visible
            .iter()
            .position(|child| snapshot.nodes[*child].parent == Some(index))
        {
            self.set_selection(position);
        }
    }

    fn collapse_or_parent(&mut self) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let visible = self.visible();
        let Some(index) = visible.get(self.selected_visible).copied() else {
            return;
        };
        let node = &snapshot.nodes[index];
        if node.child_count > 0 && !self.collapsed.contains(&node.identity) {
            self.collapsed.insert(node.identity.clone());
            return;
        }
        if let Some(parent) = node.parent
            && let Some(position) = visible.iter().position(|index| *index == parent)
        {
            self.set_selection(position);
        }
    }

    fn find_next(&mut self, reverse: bool) {
        if self.find.is_empty() {
            return;
        }
        let visible = self.visible();
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let needle = self.find.to_lowercase();
        let matches: Vec<usize> = visible
            .iter()
            .enumerate()
            .filter_map(|(position, index)| {
                let node = &snapshot.nodes[*index];
                format!("{} {}", node.identity, node.status)
                    .to_lowercase()
                    .contains(&needle)
                    .then_some(position)
            })
            .collect();
        if matches.is_empty() {
            self.status = format!("No matches for {:?}", self.find);
            return;
        }
        let next = if reverse {
            matches
                .iter()
                .rev()
                .copied()
                .find(|position| *position < self.selected_visible)
                .unwrap_or_else(|| *matches.last().expect("matches is not empty"))
        } else {
            matches
                .iter()
                .copied()
                .find(|position| *position > self.selected_visible)
                .unwrap_or(matches[0])
        };
        self.set_selection(next);
    }

    fn submit_input(&mut self) {
        match self.mode {
            InputMode::Filter => self.filter = self.input.trim().to_owned(),
            InputMode::Find => {
                self.find = self.input.trim().to_owned();
                self.find_next(false);
            }
            InputMode::Normal | InputMode::Help => {}
        }
        self.mode = InputMode::Normal;
        self.input.clear();
        self.set_selection(self.selected_visible);
    }

    fn handle_mouse(&mut self, mouse: MouseEvent, terminal_area: Rect) -> Option<String> {
        let Some(Modal::Text {
            content,
            kind,
            wrapped,
            vertical_scroll,
            horizontal_scroll,
            selection,
            ..
        }) = &mut self.modal
        else {
            return None;
        };
        if !kind.supports_mouse_selection() {
            return None;
        }
        let body = content_modal_body(terminal_area, *kind);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                *selection = selection_point_at(
                    content,
                    body,
                    mouse.column,
                    mouse.row,
                    *vertical_scroll,
                    *horizontal_scroll,
                    *wrapped,
                    false,
                )
                .map(|point| TextSelection {
                    anchor: point,
                    focus: point,
                });
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(active) = selection
                    && let Some(point) = selection_point_at(
                        content,
                        body,
                        mouse.column,
                        mouse.row,
                        *vertical_scroll,
                        *horizontal_scroll,
                        *wrapped,
                        true,
                    )
                {
                    active.focus = point;
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let mut active = selection.take()?;
                if let Some(point) = selection_point_at(
                    content,
                    body,
                    mouse.column,
                    mouse.row,
                    *vertical_scroll,
                    *horizontal_scroll,
                    *wrapped,
                    true,
                ) {
                    active.focus = point;
                }
                let range = active.range();
                if !range.is_empty() {
                    return Some(content[range].to_owned());
                }
            }
            _ => {}
        }
        None
    }

    fn handle_key(&mut self, key: KeyEvent, terminal_area: Rect) -> UiAction {
        let page_size = terminal_area.height.saturating_sub(6) as usize;
        if key.kind != KeyEventKind::Press {
            return UiAction::None;
        }
        if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
            self.quit = true;
            return UiAction::None;
        }
        if self.resource_missing {
            return match key.code {
                KeyCode::Char('q') => {
                    self.quit = true;
                    UiAction::None
                }
                KeyCode::Char('r') => UiAction::Refresh,
                _ => UiAction::None,
            };
        }
        if let Some(modal) = &mut self.modal {
            match modal {
                Modal::Text {
                    content,
                    kind,
                    wrapped,
                    vertical_scroll,
                    horizontal_scroll,
                    query,
                    search_input,
                    ..
                } => {
                    let modal_area = content_modal_area(terminal_area, *kind);
                    let body_width = modal_area.width.saturating_sub(2) as usize;
                    let body_height = modal_area.height.saturating_sub(3) as usize;
                    let max_vertical =
                        modal_max_vertical(content, body_width, body_height, *wrapped);
                    let max_horizontal = if *wrapped {
                        0
                    } else {
                        modal_max_horizontal(content, body_width)
                    };
                    if let Some(input) = search_input {
                        match key.code {
                            KeyCode::Esc => *search_input = None,
                            KeyCode::Enter => {
                                *query = input.trim().to_owned();
                                *search_input = None;
                                move_modal_match(
                                    content,
                                    query,
                                    vertical_scroll,
                                    false,
                                    body_width,
                                    *wrapped,
                                );
                            }
                            KeyCode::Backspace => {
                                input.pop();
                            }
                            KeyCode::Char(character) => input.push(character),
                            _ => {}
                        }
                    } else {
                        match (key.code, key.modifiers) {
                            (KeyCode::Esc | KeyCode::Char('q'), _) => self.modal = None,
                            (KeyCode::Down | KeyCode::Char('j'), _) => {
                                *vertical_scroll =
                                    vertical_scroll.saturating_add(1).min(max_vertical);
                            }
                            (KeyCode::Up | KeyCode::Char('k'), _) => {
                                *vertical_scroll = vertical_scroll.saturating_sub(1);
                            }
                            (KeyCode::PageDown, _) => {
                                *vertical_scroll = vertical_scroll
                                    .saturating_add(body_height.try_into().unwrap_or(u16::MAX))
                                    .min(max_vertical);
                            }
                            (KeyCode::PageUp, _) => {
                                *vertical_scroll = vertical_scroll
                                    .saturating_sub(page_size.try_into().unwrap_or(u16::MAX));
                            }
                            (KeyCode::Home | KeyCode::Char('g'), _) => *vertical_scroll = 0,
                            (KeyCode::End | KeyCode::Char('G'), _) => {
                                *vertical_scroll = max_vertical;
                            }
                            (KeyCode::Left, _) | (KeyCode::Char('h'), KeyModifiers::NONE) => {
                                *horizontal_scroll = horizontal_scroll.saturating_sub(4);
                            }
                            (KeyCode::Right, _) | (KeyCode::Char('l'), KeyModifiers::NONE) => {
                                *horizontal_scroll =
                                    horizontal_scroll.saturating_add(4).min(max_horizontal);
                            }
                            (KeyCode::Char('w'), KeyModifiers::NONE)
                                if *kind == ContentKind::Yaml =>
                            {
                                *wrapped = !*wrapped;
                                *vertical_scroll = 0;
                                *horizontal_scroll = 0;
                            }
                            (KeyCode::Char('/'), _) => *search_input = Some(query.clone()),
                            (KeyCode::Char('n'), _) => {
                                move_modal_match(
                                    content,
                                    query,
                                    vertical_scroll,
                                    false,
                                    body_width,
                                    *wrapped,
                                );
                            }
                            (KeyCode::Char('N'), _) => {
                                move_modal_match(
                                    content,
                                    query,
                                    vertical_scroll,
                                    true,
                                    body_width,
                                    *wrapped,
                                );
                            }
                            _ => {}
                        }
                    }
                }
                Modal::Delete {
                    target,
                    propagation,
                } => match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => self.modal = None,
                    KeyCode::Char('c') if key.modifiers == KeyModifiers::NONE => {
                        *propagation = propagation.next();
                    }
                    KeyCode::Enter => {
                        let action = UiAction::Delete(target.clone(), *propagation);
                        self.modal = None;
                        return action;
                    }
                    _ => {}
                },
                Modal::Finalizers {
                    target,
                    finalizers,
                    selected,
                    cursor,
                } => match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => self.modal = None,
                    KeyCode::Down | KeyCode::Char('j') => {
                        *cursor = cursor
                            .saturating_add(1)
                            .min(finalizers.len().saturating_sub(1));
                    }
                    KeyCode::Up | KeyCode::Char('k') => *cursor = cursor.saturating_sub(1),
                    KeyCode::Char(' ') => {
                        if !selected.remove(cursor) {
                            selected.insert(*cursor);
                        }
                    }
                    KeyCode::Enter => {
                        let selected = selected
                            .iter()
                            .filter_map(|index| finalizers.get(*index).cloned())
                            .collect();
                        let action = UiAction::RemoveFinalizers(target.clone(), selected);
                        self.modal = None;
                        return action;
                    }
                    _ => {}
                },
            }
            return UiAction::None;
        }
        if self.mode == InputMode::Help {
            let area = centered(terminal_area, 72, 20);
            let body_height = area.height.saturating_sub(3);
            let max_scroll = (HELP_LINES.len() as u16).saturating_sub(body_height);
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                    self.mode = InputMode::Normal;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.help_scroll = self.help_scroll.saturating_add(1).min(max_scroll);
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.help_scroll = self.help_scroll.saturating_sub(1);
                }
                KeyCode::PageDown => {
                    self.help_scroll = self.help_scroll.saturating_add(body_height).min(max_scroll);
                }
                KeyCode::PageUp => {
                    self.help_scroll = self.help_scroll.saturating_sub(body_height);
                }
                KeyCode::Home | KeyCode::Char('g') => self.help_scroll = 0,
                KeyCode::End | KeyCode::Char('G') => self.help_scroll = max_scroll,
                _ => {}
            }
            return UiAction::None;
        }
        if matches!(self.mode, InputMode::Filter | InputMode::Find) {
            match key.code {
                KeyCode::Esc => {
                    self.mode = InputMode::Normal;
                    self.input.clear();
                }
                KeyCode::Enter => self.submit_input(),
                KeyCode::Backspace => {
                    self.input.pop();
                }
                KeyCode::Char(character) => self.input.push(character),
                _ => {}
            }
            self.ensure_selection_visible(page_size);
            return UiAction::None;
        }

        match (key.code, key.modifiers) {
            (KeyCode::Char('q'), _) => {
                self.quit = true;
            }
            (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                if self.config.read_only {
                    self.status = "Delete is disabled in read-only mode".into();
                } else if let Some(target) = self.selected_target() {
                    self.modal = Some(Modal::Delete {
                        target,
                        propagation: DeletePropagation::Foreground,
                    });
                }
            }
            (KeyCode::Char('x'), KeyModifiers::CONTROL) => {
                if self.config.read_only {
                    self.status = "Finalizer removal is disabled in read-only mode".into();
                } else if let Some(target) = self.selected_target() {
                    let finalizers = self
                        .selected_node()
                        .and_then(|node| node.object.pointer("/metadata/finalizers"))
                        .and_then(serde_json::Value::as_array)
                        .map(|values| {
                            values
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .map(str::to_owned)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    if finalizers.is_empty() {
                        self.status = "Selected resource has no finalizers".into();
                    } else {
                        self.modal = Some(Modal::Finalizers {
                            target,
                            selected: (0..finalizers.len()).collect(),
                            finalizers,
                            cursor: 0,
                        });
                    }
                }
            }
            (KeyCode::Down | KeyCode::Char('j'), _) => self.move_selection(1),
            (KeyCode::Up | KeyCode::Char('k'), _) => self.move_selection(-1),
            (KeyCode::PageDown, _) | (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
                self.move_selection(page_size.cast_signed());
            }
            (KeyCode::PageUp, _) | (KeyCode::Char('b'), KeyModifiers::CONTROL) => {
                self.move_selection(-page_size.cast_signed());
            }
            (KeyCode::Home | KeyCode::Char('g'), _) => self.set_selection(0),
            (KeyCode::End | KeyCode::Char('G'), _) => {
                self.set_selection(self.visible().len().saturating_sub(1));
            }
            (KeyCode::Enter | KeyCode::Char(' '), _) => self.toggle_selected(),
            (KeyCode::Right, _) => self.expand_or_child(),
            (KeyCode::Left, _) => self.collapse_or_parent(),
            (KeyCode::Char(']'), _) => self.collapsed.clear(),
            (KeyCode::Char('['), _) => {
                if let Some(snapshot) = &self.snapshot {
                    self.collapsed.extend(
                        snapshot
                            .nodes
                            .iter()
                            .filter(|node| node.child_count > 0)
                            .map(|node| node.identity.clone()),
                    );
                }
                self.set_selection(0);
            }
            (KeyCode::Char('/'), _) => {
                self.mode = InputMode::Filter;
                self.input.clone_from(&self.filter);
            }
            (KeyCode::Char('f'), _) => {
                self.mode = InputMode::Find;
                self.input.clone_from(&self.find);
            }
            (KeyCode::Char('n'), _) => self.find_next(false),
            (KeyCode::Char('N'), _) => self.find_next(true),
            (KeyCode::Esc, _) => {
                self.filter.clear();
                self.find.clear();
                self.set_selection(self.selected_visible);
            }
            (KeyCode::Char('?'), _) => {
                self.help_scroll = 0;
                self.mode = InputMode::Help;
            }
            (KeyCode::Char('P'), _) => {
                self.paused = !self.paused;
                self.status = if self.paused {
                    "Automatic refresh paused".into()
                } else {
                    "Automatic refresh resumed".into()
                };
            }
            (KeyCode::Char('d'), _) => {
                if let Some(target) = self.selected_target() {
                    return UiAction::Describe(target);
                }
            }
            (KeyCode::Char('y'), _) => {
                if let Some(target) = self.selected_target() {
                    return UiAction::Yaml(target);
                }
            }
            (KeyCode::Char('v'), _) => {
                if let Some(target) = self.selected_target() {
                    return UiAction::Events(target);
                }
            }
            (KeyCode::Char('e'), _) => {
                if self.config.read_only {
                    self.status = "Edit is disabled in read-only mode".into();
                } else if let Some(target) = self.selected_target() {
                    return UiAction::Edit(target);
                }
            }
            (KeyCode::Char('c'), _) => {
                if let Some(target) = self.selected_target() {
                    return UiAction::Copy(target.identity.to_string());
                }
            }
            (KeyCode::Char('z'), _) => {
                self.full_width = !self.full_width;
                self.horizontal_offset = 0;
                self.status = if self.full_width {
                    "Full-width mode; use Alt+h/Alt+l to scroll".into()
                } else {
                    "Fitted column mode".into()
                };
            }
            (KeyCode::Char('h'), KeyModifiers::ALT) => {
                self.horizontal_offset = self.horizontal_offset.saturating_sub(4);
            }
            (KeyCode::Char('l'), KeyModifiers::ALT) => {
                self.horizontal_offset = self.horizontal_offset.saturating_add(4);
            }
            (KeyCode::Char('p'), _) => {
                if self.config.read_only {
                    self.status = "Pause is disabled in read-only mode".into();
                } else if let Some(target) = self.selected_target() {
                    return UiAction::SetPaused(target, true);
                }
            }
            (KeyCode::Char('u'), _) => {
                if self.config.read_only {
                    self.status = "Unpause is disabled in read-only mode".into();
                } else if let Some(target) = self.selected_target() {
                    return UiAction::SetPaused(target, false);
                }
            }
            (KeyCode::Char('r'), _) => return UiAction::Refresh,
            _ => {}
        }
        self.ensure_selection_visible(page_size);
        UiAction::None
    }
}

#[derive(Clone, Debug)]
enum UiAction {
    None,
    Refresh,
    Describe(Target),
    Yaml(Target),
    Events(Target),
    Edit(Target),
    Copy(String),
    Delete(Target, DeletePropagation),
    SetPaused(Target, bool),
    RemoveFinalizers(Target, Vec<String>),
}

pub async fn run(cli: &Cli, resource: String, config: Config) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(anyhow!("xpdelve requires an interactive terminal"));
    }
    if !trace::executable_available(&config.trace.program).unwrap_or(false) {
        return Err(anyhow!(
            "trace executable {:?} was not found in PATH; install the Crossplane CLI or configure trace.program",
            config.trace.program
        ));
    }

    let theme = Theme::resolve(&config.skin, config.ui.color)?;
    let mut terminal = TerminalGuard::enter()?;
    let (sender, mut receiver) = mpsc::channel(EVENT_BUFFER);
    let mut events = EventStream::new();
    let mut refresh =
        time::interval_at(time::Instant::now() + config.interval(), config.interval());
    refresh.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    let mut app = App::new(resource, config, cli, theme);
    let mut active = None;
    connect_kubernetes(cli, &sender);
    request_refresh(&mut app, cli, &sender, &mut active, true);

    while !app.quit {
        terminal.set_mouse_capture(matches!(
            &app.modal,
            Some(Modal::Text { kind, .. }) if kind.supports_mouse_selection()
        ))?;
        terminal.terminal.draw(|frame| render(frame, &app))?;
        let toast_active = app.toast.is_some();
        let toast_delay = app
            .toast
            .as_ref()
            .map_or(Duration::from_secs(86_400), |toast| {
                toast.expires_at.saturating_duration_since(Instant::now())
            });
        tokio::select! {
            event = events.next() => {
                let Some(event) = event else { break };
                match event.context("failed to read terminal event")? {
                    TerminalEvent::Key(key) => {
                        let area = terminal.terminal.size()?;
                        match app.handle_key(key, area.into()) {
                            UiAction::None => {}
                            UiAction::Refresh => request_refresh(&mut app, cli, &sender, &mut active, true),
                            UiAction::Copy(value) => {
                                app.status = copy_osc52(&value).map_or_else(
                                    |error| format!("Copy failed: {error}"),
                                    |()| format!("Copied {value}"),
                                );
                            }
                            UiAction::Edit(target) => {
                                let success = run_kubectl(&mut terminal, &app, cli, target, "edit").await;
                                let should_refresh = success.is_ok();
                                app.status = match success {
                                    Ok(()) => "Edit completed".into(),
                                    Err(error) => format!("Edit failed: {error}"),
                                };
                                if should_refresh {
                                    request_refresh(&mut app, cli, &sender, &mut active, false);
                                }
                            }
                            action => start_action(&mut app, action, &sender, cli),
                        }
                    }
                    TerminalEvent::Mouse(mouse) => {
                        let area = terminal.terminal.size()?;
                        if let Some(value) = app.handle_mouse(mouse, area.into()) {
                            match copy_osc52(&value) {
                                Ok(()) => app.show_toast("● Copied to clipboard"),
                                Err(error) => app.status = format!("Copy failed: {error}"),
                            }
                        }
                    }
                    _ => {}
                }
            }
            Some(event) = receiver.recv() => {
                match event {
                    AppEvent::TraceFinished { generation, result } if generation == app.generation => {
                        active = None;
                        let refresh_pending = std::mem::take(&mut app.refresh_pending);
                        match result {
                            TraceResult::Snapshot(snapshot) => app.apply_snapshot(snapshot),
                            TraceResult::ResourceNotFound => app.apply_resource_not_found(),
                            TraceResult::Failed(error) => {
                                app.loading = false;
                                app.status = error;
                                if !refresh_pending
                                    && !app.no_watch
                                    && !app.paused
                                    && !app.resource_missing
                                {
                                    schedule_retry(&app, &sender);
                                    app.retry_delay = (app.retry_delay * 2).min(Duration::from_secs(app.config.trace.retry_backoff_max_seconds));
                                }
                            }
                        }
                        if refresh_pending && !app.resource_missing {
                            request_refresh(&mut app, cli, &sender, &mut active, false);
                        }
                    }
                    AppEvent::TraceFinished { .. } => {}
                    AppEvent::KubernetesReady(result) => match result {
                        Ok(kubernetes) => {
                            app.kubernetes = Some(kubernetes);
                            app.kubernetes_status = "Kubernetes client ready".into();
                        }
                        Err(error) => {
                            app.kubernetes_status = format!("Kubernetes actions unavailable: {error}");
                        }
                    },
                    AppEvent::ActionFinished { label, identity, result, refresh } => {
                        if let Some(identity) = identity {
                            app.active_mutations.remove(&identity);
                        }
                        let succeeded = result.is_ok();
                        match result {
                            Ok(Some(content)) => {
                                let kind = match label.as_str() {
                                    label if label.starts_with("YAML:") => ContentKind::Yaml,
                                    label if label.starts_with("Events:") => ContentKind::Events,
                                    _ => ContentKind::Describe,
                                };
                                app.modal = Some(Modal::Text {
                                    title: label.clone(),
                                    content,
                                    kind,
                                    wrapped: content_wraps_by_default(kind),
                                    vertical_scroll: 0,
                                    horizontal_scroll: 0,
                                    query: String::new(),
                                    search_input: None,
                                    selection: None,
                                });
                            }
                            Ok(None) => app.status = format!("{label} succeeded"),
                            Err(error) => {
                                app.status = text::sanitize(&format!("{label} failed: {error}"));
                            }
                        }
                        if refresh && succeeded {
                            request_refresh(&mut app, cli, &sender, &mut active, false);
                            app.status = format!("{label} succeeded; refreshing trace...");
                        }
                    }
                    AppEvent::Retry { generation }
                        if generation == app.generation
                            && !app.loading
                            && !app.paused
                            && !app.no_watch
                            && !app.resource_missing => {
                        request_refresh(&mut app, cli, &sender, &mut active, false);
                    }
                    AppEvent::Retry { .. } => {}
                }
            }
            _ = refresh.tick(), if !app.no_watch && !app.paused && !app.loading && !app.resource_missing => {
                request_refresh(&mut app, cli, &sender, &mut active, false);
            }
            _ = time::sleep(toast_delay), if toast_active => {
                app.toast = None;
            }
        }
    }
    if let Some(token) = active {
        token.cancel();
        time::sleep(Duration::from_millis(800)).await;
    }
    Ok(())
}

fn connect_kubernetes(cli: &Cli, sender: &mpsc::Sender<AppEvent>) {
    let kubeconfig = cli.kubeconfig.clone();
    let context = cli.context.clone();
    let sender = sender.clone();
    tokio::spawn(async move {
        let result = Kubernetes::connect(kubeconfig.as_deref(), context.as_deref())
            .await
            .map_err(|error| error.to_string());
        let _ = sender.send(AppEvent::KubernetesReady(result)).await;
    });
}

fn start_action(app: &mut App, action: UiAction, sender: &mpsc::Sender<AppEvent>, cli: &Cli) {
    let Some(kubernetes) = app.kubernetes.clone() else {
        app.status.clone_from(&app.kubernetes_status);
        return;
    };
    if let Some(identity) = mutation_identity(&action) {
        if app.active_mutations.len() >= 4 {
            app.status = "Four mutations are already running; wait for one to finish".into();
            return;
        }
        if !app.active_mutations.insert(identity.clone()) {
            app.status = format!("An action is already running for {identity}");
            return;
        }
    }
    let identity = mutation_identity(&action).cloned();
    let context = cli.context.clone();
    let kubeconfig = cli.kubeconfig.clone();
    let sender = sender.clone();
    app.status = "Kubernetes operation in progress...".into();
    tokio::spawn(async move {
        let (label, result, refresh) = match action {
            UiAction::Describe(target) => (
                format!("Describe: {}", target.identity),
                capture_kubectl_describe(
                    &kubernetes,
                    &target,
                    context.as_deref(),
                    kubeconfig.as_deref(),
                )
                .await
                .map(Some),
                false,
            ),
            UiAction::Yaml(target) => (
                format!("YAML: {}", target.identity),
                kubernetes.yaml(&target).await.map(Some),
                false,
            ),
            UiAction::Events(target) => (
                format!("Events: {}", target.identity),
                kubernetes.events(&target).await.map(Some),
                false,
            ),
            UiAction::Delete(target, propagation) => (
                format!("Delete ({propagation})"),
                kubernetes.delete(&target, propagation).await.map(|()| None),
                true,
            ),
            UiAction::SetPaused(target, paused) => (
                if paused { "Pause" } else { "Unpause" }.into(),
                kubernetes.set_paused(&target, paused).await.map(|()| None),
                true,
            ),
            UiAction::RemoveFinalizers(target, selected) => (
                "Remove finalizers".into(),
                kubernetes
                    .remove_finalizers(&target, &selected)
                    .await
                    .map(|()| None),
                true,
            ),
            UiAction::None | UiAction::Refresh | UiAction::Edit(_) | UiAction::Copy(_) => return,
        };
        let result = result.map_err(|error| error.to_string());
        let _ = sender
            .send(AppEvent::ActionFinished {
                label,
                identity,
                result,
                refresh,
            })
            .await;
    });
}

fn mutation_identity(action: &UiAction) -> Option<&Identity> {
    match action {
        UiAction::Delete(target, _)
        | UiAction::SetPaused(target, _)
        | UiAction::RemoveFinalizers(target, _) => Some(&target.identity),
        _ => None,
    }
}

fn copy_osc52(value: &str) -> Result<()> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(value);
    let mut stdout = io::stdout();
    write!(stdout, "\x1b]52;c;{encoded}\x07")?;
    stdout.flush()?;
    Ok(())
}

async fn run_kubectl(
    terminal: &mut TerminalGuard,
    app: &App,
    cli: &Cli,
    target: Target,
    operation: &str,
) -> Result<()> {
    let kubernetes = app
        .kubernetes
        .as_ref()
        .context("Kubernetes client is unavailable")?;
    let resource = kubernetes.kubectl_resource(&target).await?;
    terminal.suspend()?;
    let mut command = tokio::process::Command::new("kubectl");
    command.args(kubectl_args(
        operation,
        &resource,
        target.identity.namespace.as_deref(),
        cli.context.as_deref(),
        cli.kubeconfig.as_deref(),
    ));
    let result = command
        .status()
        .await
        .with_context(|| format!("failed to start kubectl {operation}"));
    terminal.resume()?;
    let status = result?;
    if !status.success() {
        return Err(anyhow!("kubectl {operation} exited with {status}"));
    }
    Ok(())
}

async fn capture_kubectl_describe(
    kubernetes: &Kubernetes,
    target: &Target,
    context: Option<&str>,
    kubeconfig: Option<&std::path::Path>,
) -> Result<String> {
    let resource = kubernetes.kubectl_resource(target).await?;
    let mut command = tokio::process::Command::new("kubectl");
    command.args(kubectl_args(
        "describe",
        &resource,
        target.identity.namespace.as_deref(),
        context,
        kubeconfig,
    ));
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .context("failed to start kubectl describe")?;
    let stdout = child
        .stdout
        .take()
        .context("kubectl stdout was not captured")?;
    let stderr = child
        .stderr
        .take()
        .context("kubectl stderr was not captured")?;
    let stdout = tokio::spawn(read_limited(stdout, DESCRIBE_OUTPUT_LIMIT, "stdout"));
    let stderr = tokio::spawn(read_limited(stderr, DESCRIBE_ERROR_LIMIT, "stderr"));
    let status = time::timeout(DESCRIBE_TIMEOUT, child.wait())
        .await
        .map_err(|_| anyhow!("kubectl describe timed out after 60 seconds"))?
        .context("failed while waiting for kubectl describe")?;
    let stdout = stdout.await.context("kubectl stdout reader stopped")??;
    let stderr = stderr.await.context("kubectl stderr reader stopped")??;
    if !status.success() {
        let diagnostic = String::from_utf8_lossy(&stderr);
        return Err(anyhow!(
            "kubectl describe exited with {}: {}",
            status,
            diagnostic.trim()
        ));
    }
    String::from_utf8(stdout)
        .map(|output| text::sanitize(&output))
        .context("kubectl describe returned non-UTF-8 output")
}

async fn read_limited(
    mut reader: impl tokio::io::AsyncRead + Unpin,
    limit: usize,
    stream: &'static str,
) -> Result<Vec<u8>> {
    let mut output = Vec::with_capacity(limit.min(64 * 1024));
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        if output.len().saturating_add(read) > limit {
            bail!("kubectl describe {stream} exceeded its {limit} byte limit");
        }
        output.extend_from_slice(&buffer[..read]);
    }
    Ok(output)
}

fn kubectl_args(
    operation: &str,
    resource: &str,
    namespace: Option<&str>,
    context: Option<&str>,
    kubeconfig: Option<&std::path::Path>,
) -> Vec<std::ffi::OsString> {
    let mut args = vec![operation.into(), resource.into()];
    if let Some(namespace) = namespace {
        args.extend(["--namespace".into(), namespace.into()]);
    }
    if let Some(context) = context {
        args.extend(["--context".into(), context.into()]);
    }
    if let Some(kubeconfig) = kubeconfig {
        args.push("--kubeconfig".into());
        args.push(kubeconfig.as_os_str().to_owned());
    }
    args
}

fn request_refresh(
    app: &mut App,
    cli: &Cli,
    sender: &mpsc::Sender<AppEvent>,
    active: &mut Option<CancellationToken>,
    cancel_active: bool,
) {
    if let Some(token) = active.as_ref() {
        app.refresh_pending = true;
        if cancel_active {
            app.status = "Cancelling current trace...".into();
            token.cancel();
        }
        return;
    }
    app.generation = app.generation.wrapping_add(1);
    app.loading = true;
    app.status = "Refreshing trace...".into();
    let generation = app.generation;
    let token = CancellationToken::new();
    *active = Some(token.clone());
    let sender = sender.clone();
    let config = app.config.trace.clone();
    let request = TraceRequest {
        resource: app.resource.clone(),
        context: cli.context.clone(),
        namespace: cli.namespace.clone(),
        kubeconfig: cli.kubeconfig.clone(),
        timeout: app.config.timeout(),
    };
    tokio::spawn(async move {
        let result = match trace::execute(&config, &request, token.clone()).await {
            Ok(output) => {
                let parse = tokio::task::spawn_blocking(move || Snapshot::parse(&output.stdout));
                tokio::select! {
                    result = parse => result
                        .map_err(|error| format!("trace parser stopped unexpectedly: {error}"))
                        .and_then(|result| result.map_err(|error| error.to_string()))
                        .map_or_else(TraceResult::Failed, TraceResult::Snapshot),
                    () = token.cancelled() => TraceResult::Failed("trace refresh cancelled".into()),
                }
            }
            Err(trace::TraceError::ResourceNotFound { .. }) => TraceResult::ResourceNotFound,
            Err(trace::TraceError::Other(error)) => TraceResult::Failed(error.to_string()),
        };
        let _ = sender
            .send(AppEvent::TraceFinished { generation, result })
            .await;
    });
}

fn schedule_retry(app: &App, sender: &mpsc::Sender<AppEvent>) {
    let generation = app.generation;
    let delay = app.retry_delay;
    let sender = sender.clone();
    tokio::spawn(async move {
        time::sleep(delay).await;
        let _ = sender.send(AppEvent::Retry { generation }).await;
    });
}

fn render(frame: &mut ratatui::Frame<'_>, app: &App) {
    let area = frame.area();
    if area.width < 32 || area.height < 8 {
        frame.render_widget(
            Paragraph::new("Terminal too small\nResize to at least 32x8")
                .block(bordered_block(" xpdelve ", &app.theme)),
            area,
        );
        return;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);
    render_header(frame, chunks[0], app);
    render_tree(frame, chunks[1], app);
    render_prompt(frame, chunks[2], app);
    render_status(frame, chunks[3], app);
    if app.mode == InputMode::Help {
        render_help(frame, centered(area, 72, 20), app.help_scroll, &app.theme);
    }
    if let Some(modal) = &app.modal {
        let modal_area = match modal {
            Modal::Text { kind, .. } => content_modal_area(area, *kind),
            Modal::Delete { .. } => centered(area, 100, 12),
            Modal::Finalizers { finalizers, .. } => centered(
                area,
                100,
                u16::try_from(finalizers.len())
                    .unwrap_or(u16::MAX)
                    .saturating_add(10)
                    .clamp(12, 24),
            ),
        };
        render_modal(frame, modal_area, modal, &app.theme);
    }
    if let Some(toast) = &app.toast
        && toast.expires_at > Instant::now()
    {
        render_toast(frame, area, toast, &app.theme);
    }
}

fn render_toast(frame: &mut ratatui::Frame<'_>, area: Rect, toast: &Toast, theme: &Theme) {
    let width = u16::try_from(toast.message.width())
        .unwrap_or(u16::MAX)
        .saturating_add(4)
        .min(area.width.saturating_sub(2));
    let height = 3.min(area.height);
    let popup = Rect::new(
        area.x.saturating_add(area.width.saturating_sub(width) / 2),
        area.y
            .saturating_add(area.height.saturating_sub(height).saturating_sub(2)),
        width,
        height,
    );
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(toast.message.as_str())
            .alignment(Alignment::Center)
            .style(theme.title())
            .block(bordered_block("", theme)),
        popup,
    );
}

fn render_header(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let activity = if app.loading { " [refreshing]" } else { "" };
    let paused = if app.paused { " [paused]" } else { "" };
    let readonly = if app.config.read_only {
        " [read-only]"
    } else {
        ""
    };
    let activity_style = app.theme.activity();
    let warning_style = if app.theme.colors_enabled {
        app.theme.warning()
    } else {
        Style::default().bold()
    };
    let title_style = app.theme.title();
    let version = concat!("v", env!("CARGO_PKG_VERSION"));
    let app_version_width = "xpdelve ".width() + version.width();
    let regions = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(app_version_width as u16),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(" "),
            Span::raw(text::sanitize(&app.resource)),
            Span::styled(activity, activity_style),
            Span::styled(paused, warning_style),
            Span::styled(readonly, warning_style),
        ])),
        regions[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("xpdelve ", title_style),
            Span::styled(version, app.theme.subtle()),
        ]))
        .alignment(Alignment::Right),
        regions[1],
    );
}

fn render_tree(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    if app.resource_missing {
        render_resource_missing(frame, area, app);
        return;
    }
    let Some(snapshot) = &app.snapshot else {
        frame.render_widget(
            Paragraph::new("Waiting for the first complete trace...")
                .block(bordered_block(" Resources ", &app.theme)),
            area,
        );
        return;
    };
    let visible = app.visible();
    let package_trace = snapshot.nodes.first().is_some_and(|node| node.is_package);
    let viewport = area.height.saturating_sub(3) as usize;
    let start = app.resource_view_start(viewport);
    let plan = TablePlan::new(
        snapshot,
        &visible,
        area.width.saturating_sub(2) as usize,
        package_trace,
        app.config.ui.short,
        app.full_width,
        app.config.ui.ascii,
    );
    let mut lines = Vec::with_capacity(viewport + 1);
    lines.push(Line::styled(
        horizontal_slice(&plan.header(), app.horizontal_offset, plan.available),
        Style::default().add_modifier(Modifier::BOLD),
    ));
    for (visible_position, index) in visible.iter().enumerate().skip(start).take(viewport) {
        let node = &snapshot.nodes[*index];
        let content = horizontal_slice(
            &plan.row(snapshot, node, &app.collapsed),
            app.horizontal_offset,
            plan.available,
        );
        let selected = visible_position == app.selected_visible;
        let mut style = if selected {
            app.theme.selected_row()
        } else {
            app.theme.health(node.health)
        };
        if !app.find.is_empty()
            && content.to_lowercase().contains(&app.find.to_lowercase())
            && !selected
        {
            style = style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
        }
        lines.push(Line::styled(content, style));
    }
    let title = format!(" Resources ({}/{}) ", visible.len(), snapshot.nodes.len());
    frame.render_widget(
        Paragraph::new(lines).block(bordered_block(title, &app.theme)),
        area,
    );
}

fn render_resource_missing(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let scope = app.namespace.as_deref().map_or_else(
        || "the current namespace".to_owned(),
        |namespace| format!("namespace {namespace}"),
    );
    let context = app
        .context
        .as_deref()
        .map(|context| format!("Context: {context}"));
    let mut lines = vec![
        Line::styled("Resource not found", app.theme.warning().bold()),
        Line::from(""),
        Line::from(format!(
            "{} could not be found in {scope}.",
            text::sanitize(&app.resource)
        )),
        Line::from(
            "It may have been deleted or the selected context or namespace may have changed.",
        ),
    ];
    if let Some(context) = context {
        lines.push(Line::from(""));
        lines.push(Line::styled(context, app.theme.subtle()));
    }
    lines.extend([Line::from(""), Line::from("Press r to retry or q to quit.")]);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(bordered_block(" Resources (0/0) ", &app.theme)),
        area,
    );
}

fn condition_text(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "True",
        Some(false) => "False",
        None => "-",
    }
}

struct TablePlan {
    available: usize,
    object: usize,
    group: usize,
    status: usize,
    package: bool,
    timestamps: bool,
    version: usize,
    state: usize,
    ascii: bool,
}

impl TablePlan {
    fn new(
        snapshot: &Snapshot,
        visible: &[usize],
        available: usize,
        package: bool,
        short: bool,
        full_width: bool,
        ascii: bool,
    ) -> Self {
        let object = visible
            .iter()
            .map(|index| object_cell(snapshot, &snapshot.nodes[*index], ascii).width())
            .max()
            .unwrap_or(6)
            .max("OBJECT".len());
        let group = visible
            .iter()
            .map(|index| group_cell(&snapshot.nodes[*index]).width())
            .max()
            .unwrap_or(5)
            .max("GROUP".len());
        let timestamps = !short;
        let fixed = if package {
            if timestamps { 78 } else { 44 }
        } else if timestamps {
            53
        } else {
            19
        };
        let group_width = if package { 0 } else { group };
        let minimum_status = 1;
        let mut plan = Self {
            available,
            object,
            group,
            status: minimum_status,
            package,
            timestamps,
            version: usize::from(package) * 10,
            state: usize::from(package) * 8,
            ascii,
        };
        if !full_width && object + group_width + fixed + minimum_status > available {
            plan.timestamps = false;
        }
        if !full_width && package && object + plan.fixed_width() + minimum_status > available {
            plan.version = 0;
        }
        if !full_width && package && object + plan.fixed_width() + minimum_status > available {
            plan.state = 0;
        }
        let fixed = plan.fixed_width();
        if full_width {
            if package {
                plan.version = visible
                    .iter()
                    .filter_map(|index| snapshot.nodes[*index].version.as_deref())
                    .map(UnicodeWidthStr::width)
                    .max()
                    .unwrap_or_default()
                    .max("VERSION".len());
                plan.state = visible
                    .iter()
                    .filter_map(|index| snapshot.nodes[*index].state.as_deref())
                    .map(UnicodeWidthStr::width)
                    .max()
                    .unwrap_or_default()
                    .max("STATE".len());
            }
            plan.status = visible
                .iter()
                .map(|index| snapshot.nodes[*index].status.width())
                .max()
                .unwrap_or(6)
                .max("STATUS".len());
            return plan;
        }
        let mut overflow = object + group_width + fixed + minimum_status;
        if overflow > available {
            let reducible = if package { 0 } else { group.saturating_sub(4) };
            let reduction = reducible.min(overflow - available);
            plan.group -= reduction;
            overflow -= reduction;
        }
        if overflow > available {
            let reducible = object.saturating_sub(4);
            let reduction = reducible.min(overflow - available);
            plan.object -= reduction;
        }
        let group_width = if package { 0 } else { plan.group };
        plan.status = available
            .saturating_sub(plan.object + group_width + fixed)
            .max(1);
        plan
    }

    fn fixed_width(&self) -> usize {
        if self.package {
            let displayed = 3 + usize::from(self.version > 0) + usize::from(self.state > 0);
            let widths = 9 + 7 + if self.timestamps { 30 } else { 0 } + self.version + self.state;
            widths + displayed * 2
        } else if self.timestamps {
            53
        } else {
            19
        }
    }

    fn header(&self) -> String {
        if self.package {
            let mut columns = vec![("OBJECT", self.object)];
            if self.version > 0 {
                columns.push(("VERSION", self.version));
            }
            columns.push(("INSTALLED", 9));
            if self.timestamps {
                columns.push(("INSTALLED LAST", 15));
            }
            columns.push(("HEALTHY", 7));
            if self.timestamps {
                columns.push(("HEALTHY LAST", 15));
            }
            if self.state > 0 {
                columns.push(("STATE", self.state));
            }
            columns.push(("STATUS", self.status));
            format_columns(&columns)
        } else if self.timestamps {
            format_columns(&[
                ("OBJECT", self.object),
                ("GROUP", self.group),
                ("SYNCED", 6),
                ("SYNCED LAST", 15),
                ("READY", 5),
                ("READY LAST", 15),
                ("STATUS", self.status),
            ])
        } else {
            format_columns(&[
                ("OBJECT", self.object),
                ("GROUP", self.group),
                ("SYNCED", 6),
                ("READY", 5),
                ("STATUS", self.status),
            ])
        }
    }

    fn row(
        &self,
        snapshot: &Snapshot,
        node: &ProjectedNode,
        _collapsed: &HashSet<Identity>,
    ) -> String {
        let object = object_cell_with_state(snapshot, node, self.ascii);
        let group = group_cell(node);
        if self.package {
            let object = compact_object(&object, self.object);
            let version = text::sanitize(node.version.as_deref().unwrap_or("-"));
            let state = text::sanitize(node.state.as_deref().unwrap_or("-"));
            let mut columns = vec![(object.as_str(), self.object)];
            if self.version > 0 {
                columns.push((version.as_str(), self.version));
            }
            columns.push((condition_text(node.synced), 9));
            if self.timestamps {
                columns.push((node.synced_last.as_deref().unwrap_or("-"), 15));
            }
            columns.push((condition_text(node.ready), 7));
            if self.timestamps {
                columns.push((node.ready_last.as_deref().unwrap_or("-"), 15));
            }
            if self.state > 0 {
                columns.push((state.as_str(), self.state));
            }
            columns.push((&node.status, self.status));
            format_columns_owned(&columns)
        } else if self.timestamps {
            format_columns_owned(&[
                (&compact_object(&object, self.object), self.object),
                (&group, self.group),
                (condition_text(node.synced), 6),
                (node.synced_last.as_deref().unwrap_or("-"), 15),
                (condition_text(node.ready), 5),
                (node.ready_last.as_deref().unwrap_or("-"), 15),
                (&node.status, self.status),
            ])
        } else {
            format_columns_owned(&[
                (&compact_object(&object, self.object), self.object),
                (&group, self.group),
                (condition_text(node.synced), 6),
                (condition_text(node.ready), 5),
                (&node.status, self.status),
            ])
        }
    }
}

fn object_cell(snapshot: &Snapshot, node: &ProjectedNode, ascii: bool) -> String {
    object_cell_with_state(snapshot, node, ascii)
}

fn object_cell_with_state(snapshot: &Snapshot, node: &ProjectedNode, ascii: bool) -> String {
    let prefix = tree_prefix(snapshot, node, ascii);
    let paused = if node.paused { " (paused)" } else { "" };
    format!(
        "{prefix}{}/{}{paused}",
        text::sanitize(&node.identity.kind),
        text::sanitize(&node.identity.name)
    )
}

fn group_cell(node: &ProjectedNode) -> String {
    if node.identity.group.is_empty() {
        "core".into()
    } else {
        text::sanitize(&node.identity.group)
    }
}

fn format_columns(columns: &[(&str, usize)]) -> String {
    format_columns_owned(columns)
}

fn format_columns_owned(columns: &[(&str, usize)]) -> String {
    columns
        .iter()
        .map(|(value, width)| pad_or_truncate(value, *width))
        .collect::<Vec<_>>()
        .join("  ")
}

fn pad_or_truncate(value: &str, width: usize) -> String {
    let current = value.width();
    if current <= width {
        return format!("{value}{}", " ".repeat(width - current));
    }
    if width == 0 {
        return String::new();
    }
    if width == 1 {
        return "…".into();
    }
    let mut result = String::new();
    let target = width - 1;
    let mut used = 0;
    for character in value.chars() {
        let character_width = character.width().unwrap_or(0);
        if used + character_width > target {
            break;
        }
        result.push(character);
        used += character_width;
    }
    result.push('…');
    result.push_str(&" ".repeat(width.saturating_sub(result.width())));
    result
}

fn compact_object(value: &str, width: usize) -> String {
    if value.width() <= width || width < 4 {
        return value.to_owned();
    }
    let marker = &value[..value
        .char_indices()
        .nth(1)
        .map_or(value.len(), |(index, _)| index)];
    let suffix_width = width.saturating_sub(marker.width() + 1);
    let mut suffix = String::new();
    let mut used = 0;
    for character in value.chars().rev() {
        let character_width = character.width().unwrap_or(0);
        if used + character_width > suffix_width {
            break;
        }
        suffix.insert(0, character);
        used += character_width;
    }
    format!("{marker}…{suffix}")
}

fn horizontal_slice(value: &str, offset: u16, width: usize) -> String {
    let mut skipped = 0;
    let mut used = 0;
    let mut result = String::new();
    for character in value.chars() {
        let character_width = character.width().unwrap_or(0);
        if skipped + character_width <= usize::from(offset) {
            skipped += character_width;
            continue;
        }
        if used + character_width > width {
            break;
        }
        result.push(character);
        used += character_width;
    }
    result
}

fn tree_prefix(snapshot: &Snapshot, node: &ProjectedNode, ascii: bool) -> String {
    if node.depth == 0 {
        return String::new();
    }
    let mut ancestors = Vec::with_capacity(node.depth);
    let mut parent = node.parent;
    while let Some(index) = parent {
        ancestors.push(&snapshot.nodes[index]);
        parent = snapshot.nodes[index].parent;
    }
    ancestors.reverse();
    let mut prefix = String::new();
    for ancestor in ancestors.iter().skip(1) {
        if ancestor.is_last_child {
            prefix.push_str("   ");
        } else if ascii {
            prefix.push_str("|  ");
        } else {
            prefix.push_str("│  ");
        }
    }
    if ascii {
        prefix.push_str(if node.is_last_child { "`- " } else { "|- " });
    } else {
        prefix.push_str(if node.is_last_child {
            "└─ "
        } else {
            "├─ "
        });
    }
    prefix
}

fn render_prompt(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let line = if app.resource_missing {
        " r retry  q quit".into()
    } else {
        match app.mode {
        InputMode::Filter => format!(" Filter: {}_", app.input),
        InputMode::Find => format!(" Find: {}_", app.input),
        InputMode::Normal | InputMode::Help if !app.filter.is_empty() => {
            format!(" Filter: {}", app.filter)
        }
        InputMode::Normal | InputMode::Help if !app.find.is_empty() => {
            format!(" Find: {}", app.find)
        }
        InputMode::Normal | InputMode::Help => {
            " Enter/Space expand/collapse  ctrl-d delete  d describe  y YAML  v events  / filter  z width  ? help"
                .into()
        }
        }
    };
    frame.render_widget(Paragraph::new(line).style(app.theme.subtle()), area);
}

fn render_status(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    if app.resource_missing && !app.loading && app.status.is_empty() {
        frame.render_widget(Paragraph::new(""), area);
        return;
    }
    let selected = app
        .selected_node()
        .map(|node| node.identity.to_string())
        .unwrap_or_default();
    let message = if selected.is_empty() {
        format!(" {}", text::sanitize(&app.status))
    } else {
        format!(" {} | {}", selected, text::sanitize(&app.status))
    };
    let style = app.theme.subtle();
    frame.render_widget(Paragraph::new(message).style(style), area);
}

fn render_help(frame: &mut ratatui::Frame<'_>, area: Rect, scroll: u16, theme: &Theme) {
    frame.render_widget(Clear, area);
    let block = bordered_block(" Help ", theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let regions = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);
    let lines = HELP_LINES
        .iter()
        .map(|line| {
            if line.is_empty() {
                return Line::default();
            }
            if !line.starts_with(' ') {
                return Line::from(Span::styled(format!("  {line}"), theme.title()));
            }
            let (binding, description) = line.split_at(20);
            let binding = binding.trim_end();
            Line::from(vec![
                Span::raw("  "),
                Span::styled(binding.trim_start(), theme.warning()),
                Span::raw(" ".repeat(20 - binding.len())),
                Span::styled(description, theme.subtle()),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), regions[0]);
    frame.render_widget(
        Paragraph::new("j/k scroll  PgUp/PgDn page  g/G top/bottom  Esc/q/? close")
            .style(theme.subtle()),
        regions[1],
    );
}

fn render_modal(frame: &mut ratatui::Frame<'_>, area: Rect, modal: &Modal, theme: &Theme) {
    frame.render_widget(Clear, area);
    match modal {
        Modal::Text {
            title,
            content,
            kind,
            wrapped,
            vertical_scroll,
            horizontal_scroll,
            query,
            search_input,
            selection,
        } => {
            let block = bordered_block(format!(" {} ", text::sanitize(title)), theme);
            let inner = block.inner(area);
            frame.render_widget(block, area);
            let regions =
                Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);
            let lines = content
                .lines()
                .map(|line| styled_content_line(line, *kind, query, theme))
                .collect::<Vec<_>>();
            let mut paragraph =
                Paragraph::new(lines).scroll((*vertical_scroll, *horizontal_scroll));
            if *wrapped {
                paragraph = paragraph.wrap(Wrap { trim: false });
            }
            frame.render_widget(paragraph, regions[0]);
            if let Some(selection) = selection {
                render_text_selection(
                    frame,
                    regions[0],
                    content,
                    *selection,
                    *vertical_scroll,
                    *horizontal_scroll,
                    *wrapped,
                    theme,
                );
            }
            let footer = search_input.as_ref().map_or_else(
                || content_modal_footer(*kind, *wrapped).into(),
                |input| format!(" Find: {input}_"),
            );
            frame.render_widget(Paragraph::new(footer).style(theme.subtle()), regions[1]);
        }
        Modal::Delete {
            target,
            propagation,
        } => {
            let block = destructive_block(" Delete resource ", theme);
            let inner = block.inner(area);
            frame.render_widget(block, area);
            let regions =
                Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);
            let content_width = usize::from(regions[0].width);
            let (resource, metadata) = target_summary(target, content_width);
            let identity_style = theme.title();
            let selected_style = theme.selected_option();
            let subtle_style = theme.subtle();
            let policy = |candidate, name| {
                let selected = candidate == *propagation;
                Span::styled(
                    format!("{} {name}", if selected { '●' } else { '○' }),
                    if selected {
                        selected_style
                    } else {
                        subtle_style
                    },
                )
            };
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(""),
                    Line::styled(resource, identity_style),
                    Line::styled(metadata, subtle_style),
                    Line::from(""),
                    Line::styled("Propagation", Style::default().bold()),
                    Line::from(vec![
                        policy(DeletePropagation::Foreground, "Foreground"),
                        Span::raw("    "),
                        policy(DeletePropagation::Background, "Background"),
                        Span::raw("    "),
                        policy(DeletePropagation::Orphan, "Orphan"),
                    ]),
                    Line::from(""),
                    Line::styled(propagation.explanation(), subtle_style),
                    Line::from(""),
                ]),
                regions[0],
            );
            let delete_style = if theme.colors_enabled {
                theme.danger()
            } else {
                Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
            };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(" c", selected_style),
                    Span::styled(" Change propagation   ", subtle_style),
                    Span::styled("Enter Delete", delete_style),
                    Span::styled("   Esc Cancel ", subtle_style),
                ])),
                regions[1],
            );
        }
        Modal::Finalizers {
            target,
            finalizers,
            selected,
            cursor,
        } => {
            let block = destructive_block(" Remove finalizers ", theme);
            let inner = block.inner(area);
            frame.render_widget(block, area);
            let regions =
                Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);
            let (resource, metadata) = target_summary(target, usize::from(regions[0].width));
            let identity_style = theme.title();
            let selected_style = theme.selected_option();
            let subtle_style = theme.subtle();
            let mut lines = vec![
                Line::from(""),
                Line::styled(resource, identity_style),
                Line::styled(metadata, subtle_style),
                Line::from(""),
                Line::styled("Finalizers", Style::default().bold()),
            ];
            lines.extend(finalizers.iter().enumerate().map(|(index, finalizer)| {
                let cursor_selected = index == *cursor;
                let checked = selected.contains(&index);
                let style = if cursor_selected {
                    selected_style
                } else if checked {
                    Style::default()
                } else {
                    subtle_style
                };
                Line::from(vec![
                    Span::styled(if cursor_selected { "› " } else { "  " }, style),
                    Span::styled(if checked { "● " } else { "○ " }, style),
                    Span::styled(text::sanitize(finalizer), style),
                ])
            }));
            lines.push(Line::from(""));
            lines.push(Line::styled(
                "Removing finalizers can bypass cleanup and leave external resources behind.",
                subtle_style,
            ));
            frame.render_widget(Paragraph::new(lines), regions[0]);
            let remove_style = if theme.colors_enabled {
                theme.danger()
            } else {
                Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
            };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(" j/k", selected_style),
                    Span::styled(" Select   ", subtle_style),
                    Span::styled("Space", selected_style),
                    Span::styled(" Toggle   ", subtle_style),
                    Span::styled("Enter Remove", remove_style),
                    Span::styled("   Esc Cancel ", subtle_style),
                ])),
                regions[1],
            );
        }
    }
}

fn target_summary(target: &Target, width: usize) -> (String, String) {
    let resource = text::sanitize(&format!(
        "{}/{}",
        target.identity.kind, target.identity.name
    ));
    let resource = pad_or_truncate(&resource, width).trim_end().to_owned();
    let group = if target.identity.group.is_empty() {
        "core"
    } else {
        &target.identity.group
    };
    let namespace = target.identity.namespace.as_deref().unwrap_or("<cluster>");
    let metadata = pad_or_truncate(
        &text::sanitize(&format!("{group} · namespace/{namespace}")),
        width,
    )
    .trim_end()
    .to_owned();
    (resource, metadata)
}

fn styled_content_line<'a>(
    line: &'a str,
    kind: ContentKind,
    query: &str,
    theme: &Theme,
) -> Line<'a> {
    let base = match kind {
        ContentKind::Events if line.trim_start().starts_with("Warning") => {
            theme.fg(theme.palette.red).bold()
        }
        ContentKind::Describe if line.ends_with(':') => theme.fg(theme.palette.teal).bold(),
        ContentKind::Yaml => return yaml_line(line, query, theme),
        ContentKind::Describe | ContentKind::Events => Style::default(),
    };
    highlighted_line(line, query, base, theme)
}

fn content_wraps_by_default(kind: ContentKind) -> bool {
    matches!(
        kind,
        ContentKind::Describe | ContentKind::Yaml | ContentKind::Events
    )
}

fn content_modal_footer(kind: ContentKind, wrapped: bool) -> &'static str {
    match (kind, wrapped) {
        (ContentKind::Yaml, true) => {
            " drag to copy  j/k or ↑/↓ vertical  w unwrap  / find  n/N matches  Esc close"
        }
        (ContentKind::Yaml, false) => {
            " drag to copy  j/k or ↑/↓ vertical  h/l or ←/→ horizontal  w wrap  / find  n/N matches  Esc close"
        }
        (ContentKind::Describe | ContentKind::Events, true) => {
            " drag to copy  j/k or ↑/↓ vertical  / find  n/N matches  Esc close"
        }
        (ContentKind::Describe | ContentKind::Events, false) => {
            " drag to copy  j/k or ↑/↓ vertical  h/l or ←/→ horizontal  / find  n/N matches  Esc close"
        }
    }
}

fn yaml_line<'a>(line: &'a str, query: &str, theme: &Theme) -> Line<'a> {
    if case_insensitive_regex(query).is_some_and(|expression| expression.is_match(line)) {
        return highlighted_line(line, query, Style::default(), theme);
    }
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') || trimmed == "---" {
        return Line::styled(line, Style::default().add_modifier(Modifier::DIM));
    }
    if let Some(colon) = line.find(':') {
        let (key, value) = line.split_at(colon);
        return Line::from(vec![
            Span::styled(key, theme.fg(theme.palette.teal).bold()),
            Span::styled(":", Style::default().add_modifier(Modifier::DIM)),
            Span::styled(&value[1..], yaml_value_style(value[1..].trim(), theme)),
        ]);
    }
    Line::styled(line, yaml_value_style(trimmed, theme))
}

fn yaml_value_style(value: &str, theme: &Theme) -> Style {
    if value.contains("<redacted>") {
        return theme.fg(theme.palette.red).bold();
    }
    if matches!(value, "true" | "false" | "null" | "~") {
        theme.fg(theme.palette.yellow)
    } else if value.parse::<f64>().is_ok() {
        theme.fg(theme.palette.mauve)
    } else if value.starts_with(['\'', '"']) {
        theme.fg(theme.palette.green)
    } else {
        Style::default()
    }
}

fn highlighted_line<'a>(line: &'a str, query: &str, base: Style, theme: &Theme) -> Line<'a> {
    if query.is_empty() {
        return Line::styled(line, base);
    }
    let Some(expression) = case_insensitive_regex(query) else {
        return Line::styled(line, base);
    };
    let mut spans = Vec::new();
    let mut start = 0;
    for matched in expression.find_iter(line) {
        let match_start = matched.start();
        let match_end = matched.end();
        if match_start > start {
            spans.push(Span::styled(&line[start..match_start], base));
        }
        let highlight = theme.search(base);
        spans.push(Span::styled(&line[match_start..match_end], highlight));
        start = match_end;
    }
    if start < line.len() {
        spans.push(Span::styled(&line[start..], base));
    }
    Line::from(spans)
}

fn move_modal_match(
    content: &str,
    query: &str,
    scroll: &mut u16,
    reverse: bool,
    width: usize,
    wrapped: bool,
) {
    if query.is_empty() {
        return;
    }
    let Some(expression) = case_insensitive_regex(query) else {
        return;
    };
    let mut rendered_row = 0;
    let matches = content
        .lines()
        .filter_map(|line| {
            let current_row = rendered_row;
            rendered_row += if wrapped {
                line.width().max(1).div_ceil(width.max(1))
            } else {
                1
            };
            expression.is_match(line).then_some(current_row)
        })
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return;
    }
    let current = usize::from(*scroll);
    let next = if reverse {
        matches
            .iter()
            .rev()
            .copied()
            .find(|index| *index < current)
            .unwrap_or_else(|| *matches.last().expect("matches is not empty"))
    } else {
        matches
            .iter()
            .copied()
            .find(|index| *index > current)
            .unwrap_or(matches[0])
    };
    *scroll = next.try_into().unwrap_or(u16::MAX);
}

fn case_insensitive_regex(query: &str) -> Option<regex::Regex> {
    (!query.is_empty())
        .then(|| {
            RegexBuilder::new(&regex::escape(query))
                .case_insensitive(true)
                .build()
                .ok()
        })
        .flatten()
}

#[derive(Clone, Debug)]
struct VisualGrapheme {
    start: usize,
    end: usize,
    width: u16,
    whitespace: bool,
}

#[derive(Clone, Debug)]
struct VisualRow {
    graphemes: Vec<VisualGrapheme>,
    source_start: usize,
}

fn visual_rows(content: &str, width: u16, wrapped: bool) -> Vec<VisualRow> {
    if width == 0 {
        return Vec::new();
    }
    let mut rows = Vec::new();
    let mut source_offset = 0;
    for raw_line in content.split_inclusive('\n') {
        let without_newline = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        let line = without_newline
            .strip_suffix('\r')
            .unwrap_or(without_newline);
        let graphemes = UnicodeSegmentation::grapheme_indices(line, true)
            .map(|(offset, symbol)| VisualGrapheme {
                start: source_offset + offset,
                end: source_offset + offset + symbol.len(),
                width: symbol.width().try_into().unwrap_or(u16::MAX),
                whitespace: symbol.chars().all(char::is_whitespace),
            })
            .collect::<Vec<_>>();
        if wrapped {
            rows.extend(wrap_visual_line(graphemes, source_offset, width));
        } else {
            rows.push(VisualRow {
                graphemes,
                source_start: source_offset,
            });
        }
        source_offset += raw_line.len();
    }
    rows
}

// Mirrors Ratatui's WordWrapper with trim=false, while retaining source offsets
// so mouse coordinates can be translated back into the original YAML.
fn wrap_visual_line(
    graphemes: Vec<VisualGrapheme>,
    source_start: usize,
    max_width: u16,
) -> Vec<VisualRow> {
    let mut rows = Vec::new();
    let mut pending_line = Vec::new();
    let mut pending_word = Vec::new();
    let mut pending_whitespace = VecDeque::new();
    let mut line_width = 0_u16;
    let mut word_width = 0_u16;
    let mut whitespace_width = 0_u16;
    let mut non_whitespace_previous = false;

    for grapheme in graphemes {
        if grapheme.width > max_width {
            continue;
        }
        let is_whitespace = grapheme.whitespace;
        let word_found = non_whitespace_previous && is_whitespace;
        let untrimmed_overflow = pending_line.is_empty()
            && word_width
                .saturating_add(whitespace_width)
                .saturating_add(grapheme.width)
                > max_width;
        if word_found || untrimmed_overflow {
            pending_line.extend(pending_whitespace.drain(..));
            line_width = line_width.saturating_add(whitespace_width);
            pending_line.append(&mut pending_word);
            line_width = line_width.saturating_add(word_width);
            whitespace_width = 0;
            word_width = 0;
        }

        let line_full = line_width >= max_width;
        let pending_word_overflow = grapheme.width > 0
            && line_width
                .saturating_add(whitespace_width)
                .saturating_add(word_width)
                >= max_width;
        if line_full || pending_word_overflow {
            let mut remaining = max_width.saturating_sub(line_width);
            rows.push(VisualRow {
                source_start: pending_line
                    .first()
                    .map_or(source_start, |item: &VisualGrapheme| item.start),
                graphemes: std::mem::take(&mut pending_line),
            });
            line_width = 0;
            while let Some(item) = pending_whitespace.front() {
                if item.width > remaining {
                    break;
                }
                whitespace_width = whitespace_width.saturating_sub(item.width);
                remaining = remaining.saturating_sub(item.width);
                pending_whitespace.pop_front();
            }
            if is_whitespace && pending_whitespace.is_empty() {
                continue;
            }
        }

        if is_whitespace {
            whitespace_width = whitespace_width.saturating_add(grapheme.width);
            pending_whitespace.push_back(grapheme);
        } else {
            word_width = word_width.saturating_add(grapheme.width);
            pending_word.push(grapheme);
        }
        non_whitespace_previous = !is_whitespace;
    }

    pending_line.extend(pending_whitespace);
    pending_line.append(&mut pending_word);
    if !pending_line.is_empty() {
        rows.push(VisualRow {
            source_start: pending_line[0].start,
            graphemes: pending_line,
        });
    }
    if rows.is_empty() {
        rows.push(VisualRow {
            graphemes: Vec::new(),
            source_start,
        });
    }
    rows
}

#[allow(clippy::too_many_arguments)]
fn selection_point_at(
    content: &str,
    body: Rect,
    column: u16,
    row: u16,
    vertical_scroll: u16,
    horizontal_scroll: u16,
    wrapped: bool,
    clamp: bool,
) -> Option<SelectionPoint> {
    if body.is_empty() {
        return None;
    }
    let inside = column >= body.x
        && column < body.x.saturating_add(body.width)
        && row >= body.y
        && row < body.y.saturating_add(body.height);
    if !inside && !clamp {
        return None;
    }
    let column = column.clamp(body.x, body.x.saturating_add(body.width).saturating_sub(1));
    let row = row.clamp(body.y, body.y.saturating_add(body.height).saturating_sub(1));
    let visual_row = usize::from(vertical_scroll) + usize::from(row - body.y);
    let rows = visual_rows(content, body.width, wrapped);
    let visual_row = visual_row.min(rows.len().saturating_sub(1));
    let row = rows.get(visual_row)?;
    let target_column = horizontal_scroll.saturating_add(column - body.x);
    let mut current_column = 0_u16;
    for grapheme in &row.graphemes {
        let end_column = current_column.saturating_add(grapheme.width.max(1));
        if target_column < end_column {
            return Some(SelectionPoint {
                start: grapheme.start,
                end: grapheme.end,
            });
        }
        current_column = end_column;
    }
    if clamp {
        return row.graphemes.last().map_or(
            Some(SelectionPoint {
                start: row.source_start,
                end: row.source_start,
            }),
            |grapheme| {
                Some(SelectionPoint {
                    start: grapheme.start,
                    end: grapheme.end,
                })
            },
        );
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn render_text_selection(
    frame: &mut ratatui::Frame<'_>,
    body: Rect,
    content: &str,
    selection: TextSelection,
    vertical_scroll: u16,
    horizontal_scroll: u16,
    wrapped: bool,
    theme: &Theme,
) {
    let range = selection.range();
    let rows = visual_rows(content, body.width, wrapped);
    for (screen_row, row) in rows
        .iter()
        .skip(usize::from(vertical_scroll))
        .take(usize::from(body.height))
        .enumerate()
    {
        let mut visual_column = 0_u16;
        for grapheme in &row.graphemes {
            let grapheme_column = visual_column;
            visual_column = visual_column.saturating_add(grapheme.width.max(1));
            if grapheme.start >= range.end || grapheme.end <= range.start {
                continue;
            }
            for offset in 0..grapheme.width.max(1) {
                let column = grapheme_column.saturating_add(offset);
                if column < horizontal_scroll {
                    continue;
                }
                let screen_column = column - horizontal_scroll;
                if screen_column >= body.width {
                    continue;
                }
                frame.buffer_mut()[(body.x + screen_column, body.y + screen_row as u16)]
                    .set_style(theme.selected_row());
            }
        }
    }
}

fn modal_max_vertical(content: &str, width: usize, height: usize, wrapped: bool) -> u16 {
    let rows = content
        .lines()
        .map(|line| {
            if wrapped {
                line.width().max(1).div_ceil(width.max(1))
            } else {
                1
            }
        })
        .sum::<usize>();
    rows.saturating_sub(height).try_into().unwrap_or(u16::MAX)
}

fn modal_max_horizontal(content: &str, width: usize) -> u16 {
    content
        .lines()
        .map(UnicodeWidthStr::width)
        .max()
        .unwrap_or_default()
        .saturating_sub(width)
        .try_into()
        .unwrap_or(u16::MAX)
}

fn bordered_block<'a>(title: impl Into<Line<'a>>, theme: &Theme) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border())
        .title_style(theme.title())
        .title(title)
}

fn destructive_block<'a>(title: impl Into<Line<'a>>, theme: &Theme) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(theme.danger())
        .title_style(theme.danger())
        .title(title)
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

fn content_modal_area(area: Rect, kind: ContentKind) -> Rect {
    match kind {
        ContentKind::Describe | ContentKind::Yaml | ContentKind::Events => area,
    }
}

fn content_modal_body(area: Rect, kind: ContentKind) -> Rect {
    let area = content_modal_area(area, kind);
    let inner = Block::default().borders(Borders::ALL).inner(area);
    Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner)[0]
}

struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    mouse_capture: bool,
}

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode().context("failed to enable terminal raw mode")?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(error).context("failed to enter alternate screen");
        }
        let terminal = match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => terminal,
            Err(error) => {
                let mut stdout = io::stdout();
                let _ = execute!(stdout, DisableMouseCapture, LeaveAlternateScreen);
                let _ = disable_raw_mode();
                return Err(error.into());
            }
        };
        Ok(Self {
            terminal,
            mouse_capture: false,
        })
    }

    fn set_mouse_capture(&mut self, enabled: bool) -> Result<()> {
        if enabled == self.mouse_capture {
            return Ok(());
        }
        if enabled {
            execute!(self.terminal.backend_mut(), EnableMouseCapture)?;
        } else {
            execute!(self.terminal.backend_mut(), DisableMouseCapture)?;
        }
        self.mouse_capture = enabled;
        Ok(())
    }

    fn suspend(&mut self) -> Result<()> {
        disable_raw_mode()?;
        if self.mouse_capture {
            execute!(self.terminal.backend_mut(), DisableMouseCapture)?;
        }
        execute!(self.terminal.backend_mut(), LeaveAlternateScreen)?;
        self.terminal.show_cursor()?;
        Ok(())
    }

    fn resume(&mut self) -> Result<()> {
        enable_raw_mode()?;
        execute!(self.terminal.backend_mut(), EnterAlternateScreen)?;
        if self.mouse_capture {
            execute!(self.terminal.backend_mut(), EnableMouseCapture)?;
        }
        self.terminal.clear()?;
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            DisableMouseCapture,
            LeaveAlternateScreen
        );
        let _ = self.terminal.show_cursor();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Snapshot;
    use clap::Parser;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    fn app() -> App {
        app_with_theme("catppuccin-mocha", crate::config::ColorMode::Always)
    }

    fn app_with_theme(name: &str, color: crate::config::ColorMode) -> App {
        let cli = Cli::parse_from(["xpdelve", "Root/root"]);
        let mut config = Config::default();
        config.skin.name = Some(name.into());
        let theme = Theme::resolve_for_mode(
            &config.skin,
            color,
            false,
            Some(terminal_colorsaurus::ThemeMode::Dark),
        )
        .unwrap();
        let mut app = App::new("Root/root".into(), config, &cli, theme);
        app.apply_snapshot(
            Snapshot::parse(
                br#"{"object":{"apiVersion":"v1","kind":"Root","metadata":{"name":"root"}},"children":[{"object":{"apiVersion":"v1","kind":"Child","metadata":{"name":"child"}}}]}"#,
            )
            .unwrap(),
        );
        app
    }

    #[test]
    fn collapse_hides_descendants() {
        let mut app = app();
        assert_eq!(app.visible().len(), 2);
        app.toggle_selected();
        assert_eq!(app.visible().len(), 1);
    }

    #[test]
    fn right_expands_a_collapsed_node() {
        let mut app = app();
        let area = Rect::new(0, 0, 100, 20);
        app.toggle_selected();

        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), area);

        assert_eq!(app.visible().len(), 2);
        assert_eq!(app.selected_visible, 0);
    }

    #[test]
    fn right_selects_the_first_child_of_an_expanded_node() {
        let mut app = app();
        let area = Rect::new(0, 0, 100, 20);

        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), area);

        assert_eq!(app.selected_visible, 1);
        assert_eq!(app.selected_node().unwrap().identity.kind, "Child");
    }

    #[test]
    fn moving_up_only_scrolls_after_selection_leaves_viewport() {
        let mut app = app();
        let children = (0..8)
            .map(|index| {
                serde_json::json!({
                    "object": {
                        "apiVersion": "v1",
                        "kind": "Child",
                        "metadata": { "name": format!("child-{index}") }
                    }
                })
            })
            .collect::<Vec<_>>();
        let trace = serde_json::json!({
            "object": {
                "apiVersion": "v1",
                "kind": "Root",
                "metadata": { "name": "root" }
            },
            "children": children
        });
        app.apply_snapshot(Snapshot::parse(trace.to_string().as_bytes()).unwrap());
        let area = Rect::new(0, 0, 100, 9);

        app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE), area);
        assert_eq!(app.resource_scroll, 6);

        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), area);
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), area);
        assert_eq!(app.selected_visible, 6);
        assert_eq!(app.resource_scroll, 6);

        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), area);
        assert_eq!(app.selected_visible, 5);
        assert_eq!(app.resource_scroll, 5);
    }

    #[test]
    fn unicode_tree_uses_disconnected_xpdig_indentation() {
        let app = app();
        let snapshot = app.snapshot.as_ref().unwrap();
        let child = &snapshot.nodes[1];
        let cell = object_cell_with_state(snapshot, child, false);
        assert!(cell.starts_with("└─ Child/child"));

        let root = &snapshot.nodes[0];
        let cell = object_cell_with_state(snapshot, root, false);
        assert!(cell.starts_with("Root/root"));
    }

    #[test]
    fn input_stays_in_filter_mode_until_submitted() {
        let mut app = app();
        let area = Rect::new(0, 0, 100, 20);
        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE), area);
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE), area);
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), area);
        assert_eq!(app.filter, "c");
        assert_eq!(app.mode, InputMode::Normal);
    }

    #[test]
    fn selection_falls_back_to_surviving_parent() {
        let mut app = app();
        app.set_selection(1);
        app.apply_snapshot(
            Snapshot::parse(
                br#"{"object":{"apiVersion":"v1","kind":"Root","metadata":{"name":"root"}}}"#,
            )
            .unwrap(),
        );
        assert_eq!(
            app.selected_node().map(|node| node.identity.to_string()),
            Some("Root/root".into())
        );
    }

    #[test]
    fn resource_not_found_clears_stale_trace_state() {
        let mut app = app();
        app.collapsed
            .insert(app.snapshot.as_ref().unwrap().nodes[0].identity.clone());
        app.modal = Some(Modal::Text {
            title: "YAML".into(),
            content: "kind: Root".into(),
            kind: ContentKind::Yaml,
            wrapped: true,
            vertical_scroll: 0,
            horizontal_scroll: 0,
            query: String::new(),
            search_input: None,
            selection: None,
        });

        app.apply_resource_not_found();

        assert!(app.resource_missing);
        assert!(app.snapshot.is_none());
        assert!(app.selected_identity.is_none());
        assert!(app.collapsed.is_empty());
        assert!(app.modal.is_none());
        assert!(!app.loading);
        assert!(app.status.is_empty());
    }

    #[test]
    fn successful_snapshot_recovers_from_resource_not_found() {
        let mut app = app();
        app.apply_resource_not_found();
        app.apply_snapshot(
            Snapshot::parse(
                br#"{"object":{"apiVersion":"v1","kind":"Root","metadata":{"name":"root"}}}"#,
            )
            .unwrap(),
        );
        assert!(!app.resource_missing);
        assert!(app.snapshot.is_some());
    }

    #[test]
    fn resource_not_found_renders_empty_panel_and_retry_legend() {
        let mut app = app();
        app.resource = "Widget/example".into();
        app.namespace = Some("apps".into());
        app.context = Some("development".into());
        app.apply_resource_not_found();
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let rendered = terminal.backend().to_string();
        assert!(rendered.contains("Resource not found"));
        assert!(rendered.contains("Widget/example could not be found in namespace apps."));
        assert!(rendered.contains("It may have been deleted"));
        assert!(rendered.contains("Context: development"));
        assert!(rendered.contains("r retry  q quit"));
        assert!(!rendered.contains("Root/root"));
    }

    #[test]
    fn resource_not_found_allows_only_retry_and_quit() {
        let mut app = app();
        app.apply_resource_not_found();
        let area = Rect::new(0, 0, 100, 20);
        assert!(matches!(
            app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE), area),
            UiAction::Refresh
        ));
        assert!(matches!(
            app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE), area),
            UiAction::None
        ));
        assert!(app.modal.is_none());
    }

    #[test]
    fn missing_state_can_show_a_distinct_manual_retry_failure() {
        let mut app = app();
        app.apply_resource_not_found();
        app.status = "trace command timed out".into();
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        assert!(
            terminal
                .backend()
                .to_string()
                .contains("trace command timed out")
        );
    }

    #[test]
    fn ordinary_table_has_aligned_xpdig_style_columns_without_resource() {
        let app = app();
        let snapshot = app.snapshot.as_ref().unwrap();
        let visible = app.visible();
        let plan = TablePlan::new(snapshot, &visible, 140, false, false, false, false);
        let header = plan.header();
        assert!(header.contains("OBJECT"));
        assert!(header.contains("GROUP"));
        assert!(header.contains("SYNCED LAST"));
        assert!(header.contains("READY LAST"));
        assert!(header.contains("STATUS"));
        assert!(!header.contains("RESOURCE"));
    }

    #[test]
    fn narrow_table_hides_both_timestamp_columns() {
        let app = app();
        let snapshot = app.snapshot.as_ref().unwrap();
        let visible = app.visible();
        let plan = TablePlan::new(snapshot, &visible, 50, false, false, false, false);
        let header = plan.header();
        assert!(!header.contains("LAST"));
        assert!(header.contains("SYNCED"));
        assert!(header.contains("READY"));
    }

    #[test]
    fn rendered_table_keeps_a_fixed_header() {
        let app = app();
        let backend = TestBackend::new(100, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let rendered = terminal.backend().to_string();
        assert!(rendered.contains("OBJECT"));
        assert!(rendered.contains("GROUP"));
        assert!(rendered.contains("Root/root"));
    }

    #[test]
    fn header_shows_app_and_version_at_right() {
        let app = app();
        let width = 100;
        let version = concat!("v", env!("CARGO_PKG_VERSION"));
        let backend = TestBackend::new(width, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();

        let app_start = width - u16::try_from("xpdelve ".width() + version.width()).unwrap();
        let version_start = width - u16::try_from(version.width()).unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer.cell((app_start, 0)).unwrap().symbol(), "x");
        assert_eq!(
            buffer.cell((app_start, 0)).unwrap().fg,
            app.theme.palette.teal
        );
        assert_eq!(buffer.cell((version_start, 0)).unwrap().symbol(), "v");
        assert_eq!(
            buffer.cell((version_start, 0)).unwrap().fg,
            app.theme.palette.overlay1
        );
    }

    #[test]
    fn selected_row_uses_sofka_content_lavender_without_cursor() {
        let app = app();
        let backend = TestBackend::new(100, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let rendered = terminal.backend().to_string();
        assert!(!rendered.contains(">-  Root/root"));
        assert_eq!(
            terminal.backend().buffer().cell((1, 3)).unwrap().fg,
            app.theme.palette.base
        );
        assert_eq!(
            terminal.backend().buffer().cell((1, 3)).unwrap().bg,
            app.theme.palette.lavender
        );
    }

    #[test]
    fn latte_selected_row_uses_light_theme_pair() {
        let app = app_with_theme("catppuccin-latte", crate::config::ColorMode::Always);
        let backend = TestBackend::new(100, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        assert_eq!(
            terminal.backend().buffer().cell((1, 3)).unwrap().fg,
            app.theme.palette.base
        );
        assert_eq!(
            terminal.backend().buffer().cell((1, 3)).unwrap().bg,
            app.theme.palette.lavender
        );
    }

    #[test]
    fn monochrome_selected_row_uses_reverse_without_palette_colors() {
        let app = app_with_theme("catppuccin-mocha", crate::config::ColorMode::Never);
        let backend = TestBackend::new(100, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let cell = terminal.backend().buffer().cell((1, 3)).unwrap();
        assert_eq!(cell.fg, Color::Reset);
        assert_eq!(cell.bg, Color::Reset);
        assert!(cell.modifier.contains(Modifier::REVERSED));
        assert!(cell.modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn content_views_use_the_full_terminal() {
        let area = Rect::new(0, 0, 100, 40);
        assert_eq!(content_modal_area(area, ContentKind::Yaml), area);
        assert_eq!(content_modal_area(area, ContentKind::Describe), area);
        assert_eq!(content_modal_area(area, ContentKind::Events), area);
    }

    #[test]
    fn borders_use_sofka_content_lavender() {
        let theme = app().theme;
        let block = bordered_block(" Test ", &theme);
        let backend = TestBackend::new(20, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| frame.render_widget(block, frame.area()))
            .unwrap();
        assert_eq!(
            terminal.backend().buffer().cell((0, 0)).unwrap().fg,
            theme.palette.lavender
        );
        assert_eq!(
            terminal.backend().buffer().cell((2, 0)).unwrap().fg,
            theme.palette.teal
        );
    }

    #[test]
    fn delete_confirmation_uses_red_border_and_footer_legend() {
        let modal = Modal::Delete {
            target: Target {
                identity: Identity {
                    group: "very-long.example.crossplane.io".into(),
                    version: "v1".into(),
                    kind: "VeryLongResourceKind".into(),
                    namespace: Some("default".into()),
                    name: "a-resource-name-that-is-long-enough-to-require-truncation".into(),
                },
                expected_uid: Some("must-not-be-rendered".into()),
            },
            propagation: DeletePropagation::Foreground,
        };
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = centered(Rect::new(0, 0, 120, 20), 100, 12);
        let theme = app().theme;
        terminal
            .draw(|frame| render_modal(frame, area, &modal, &theme))
            .unwrap();
        let rendered = terminal.backend().to_string();
        assert!(rendered.contains("Propagation"));
        assert!(rendered.contains("● Foreground"));
        assert!(rendered.contains("○ Background"));
        assert!(rendered.contains("○ Orphan"));
        assert!(rendered.contains("Dependents are deleted before the resource."));
        assert!(rendered.contains("c Change propagation"), "{rendered}");
        assert!(rendered.contains("Enter Delete"));
        assert!(!rendered.contains("UID"));
        assert!(!rendered.contains("must-not-be-rendered"));
        assert_eq!(
            terminal.backend().buffer().cell((10, 4)).unwrap().fg,
            theme.palette.red
        );
        assert_eq!(
            terminal.backend().buffer().cell((12, 4)).unwrap().fg,
            theme.palette.red
        );
        assert_eq!(
            terminal.backend().buffer().cell((11, 14)).unwrap().fg,
            theme.palette.yellow
        );
    }

    #[test]
    fn finalizer_confirmation_matches_destructive_modal_style() {
        let modal = Modal::Finalizers {
            target: Target {
                identity: Identity {
                    group: "demo.xpdelve.io".into(),
                    version: "v1alpha1".into(),
                    kind: "DemoNetwork".into(),
                    namespace: Some("xpdelve-demo".into()),
                    name: "xpdelve-demo-network".into(),
                },
                expected_uid: Some("must-not-be-rendered".into()),
            },
            finalizers: vec![
                "demo.xpdelve.io/hold-for-recording".into(),
                "kubernetes.io/foregroundDeletion".into(),
            ],
            selected: HashSet::from([0]),
            cursor: 1,
        };
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = centered(Rect::new(0, 0, 120, 20), 100, 12);
        let theme = app().theme;
        terminal
            .draw(|frame| render_modal(frame, area, &modal, &theme))
            .unwrap();
        let rendered = terminal.backend().to_string();
        assert!(rendered.contains("Remove finalizers"), "{rendered}");
        assert!(
            rendered.contains("DemoNetwork/xpdelve-demo-network"),
            "{rendered}"
        );
        assert!(
            rendered.contains("demo.xpdelve.io · namespace/xpdelve-demo"),
            "{rendered}"
        );
        assert!(
            rendered.contains("● demo.xpdelve.io/hold-for-recording"),
            "{rendered}"
        );
        assert!(
            rendered.contains("› ○ kubernetes.io/foregroundDeletion"),
            "{rendered}"
        );
        assert!(rendered.contains("Space Toggle"), "{rendered}");
        assert!(rendered.contains("Enter Remove"), "{rendered}");
        assert!(!rendered.contains("must-not-be-rendered"));
        assert_eq!(
            terminal.backend().buffer().cell((10, 4)).unwrap().fg,
            theme.palette.red
        );
        assert_eq!(
            terminal.backend().buffer().cell((12, 4)).unwrap().fg,
            theme.palette.red
        );
        assert_eq!(
            terminal.backend().buffer().cell((11, 14)).unwrap().fg,
            theme.palette.yellow
        );
    }

    #[test]
    fn main_key_legend_uses_sofka_subtle_color() {
        let app = app();
        let backend = TestBackend::new(100, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        assert_eq!(
            terminal.backend().buffer().cell((1, 14)).unwrap().fg,
            app.theme.palette.overlay1
        );
        let rendered = terminal.backend().to_string();
        assert!(rendered.contains("Enter/Space expand/collapse"));
        assert!(rendered.contains("ctrl-d delete"));
        assert!(!rendered.contains("j/k move"));
    }

    #[test]
    fn help_modal_scrolls_to_session_and_actions_on_a_standard_terminal() {
        let mut app = app();
        app.mode = InputMode::Help;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| render(frame, &app)).unwrap();

        let rendered = terminal.backend().to_string();
        assert!(rendered.contains("Navigation"));
        assert!(!rendered.contains("Session"));
        assert!(rendered.contains("j/k scroll"));

        app.handle_key(
            KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
            Rect::new(0, 0, 80, 24),
        );
        terminal.draw(|frame| render(frame, &app)).unwrap();

        let rendered = terminal.backend().to_string();
        assert!(rendered.contains("Session"));
        assert!(rendered.contains("refresh now"));
        assert!(rendered.contains("Actions"));
        assert!(rendered.contains("remove all finalizers"));
        assert!(rendered.contains("Esc/q/? close"));
    }

    #[test]
    fn help_modal_styles_headings_keys_and_descriptions() {
        let mut app = app();
        app.mode = InputMode::Help;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| render(frame, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer.cell((7, 3)).unwrap().fg, app.theme.palette.teal);
        assert_eq!(buffer.cell((7, 4)).unwrap().fg, app.theme.palette.yellow);
        assert_eq!(buffer.cell((25, 4)).unwrap().fg, app.theme.palette.overlay1);
    }

    #[test]
    fn help_navigation_keys_scroll_without_closing() {
        let mut app = app();
        app.mode = InputMode::Help;
        let area = Rect::new(0, 0, 80, 24);

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), area);
        assert_eq!(app.mode, InputMode::Help);
        assert_eq!(app.help_scroll, 1);

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), area);
        assert_eq!(app.mode, InputMode::Normal);
    }

    #[test]
    fn yaml_key_legend_uses_sofka_subtle_color() {
        let modal = Modal::Text {
            title: "YAML: Widget/example".into(),
            content: "kind: Widget".into(),
            kind: ContentKind::Yaml,
            wrapped: true,
            vertical_scroll: 0,
            horizontal_scroll: 0,
            query: String::new(),
            search_input: None,
            selection: None,
        };
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = app().theme;
        terminal
            .draw(|frame| render_modal(frame, frame.area(), &modal, &theme))
            .unwrap();
        assert_eq!(
            terminal.backend().buffer().cell((1, 18)).unwrap().fg,
            theme.palette.overlay1
        );
    }

    #[test]
    fn content_footer_groups_navigation_before_other_actions() {
        assert_eq!(
            content_modal_footer(ContentKind::Yaml, false),
            " drag to copy  j/k or ↑/↓ vertical  h/l or ←/→ horizontal  w wrap  / find  n/N matches  Esc close"
        );
        assert_eq!(
            content_modal_footer(ContentKind::Yaml, true),
            " drag to copy  j/k or ↑/↓ vertical  w unwrap  / find  n/N matches  Esc close"
        );
        assert_eq!(
            content_modal_footer(ContentKind::Events, true),
            " drag to copy  j/k or ↑/↓ vertical  / find  n/N matches  Esc close"
        );
    }

    #[test]
    fn yaml_modal_wraps_long_lines() {
        let modal = Modal::Text {
            title: "YAML: Widget/example".into(),
            content: "value: abcdefghijklmnopqrstuvwxyz".into(),
            kind: ContentKind::Yaml,
            wrapped: true,
            vertical_scroll: 0,
            horizontal_scroll: 0,
            query: String::new(),
            search_input: None,
            selection: None,
        };
        let backend = TestBackend::new(20, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = app().theme;

        terminal
            .draw(|frame| render_modal(frame, frame.area(), &modal, &theme))
            .unwrap();

        assert!(terminal.backend().to_string().contains("uvwxyz"));
    }

    #[test]
    fn yaml_modal_toggles_wrapping_and_scrolls_horizontally() {
        let mut app = app();
        app.modal = Some(Modal::Text {
            title: "YAML: Widget/example".into(),
            content: "value: abcdefghijklmnopqrstuvwxyz".into(),
            kind: ContentKind::Yaml,
            wrapped: true,
            vertical_scroll: 0,
            horizontal_scroll: 0,
            query: String::new(),
            search_input: None,
            selection: None,
        });

        app.handle_key(
            KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            Rect::new(0, 0, 20, 8),
        );
        app.handle_key(
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE),
            Rect::new(0, 0, 20, 8),
        );
        app.handle_key(
            KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            Rect::new(0, 0, 20, 8),
        );
        app.handle_key(
            KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            Rect::new(0, 0, 20, 8),
        );
        app.handle_key(
            KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
            Rect::new(0, 0, 20, 8),
        );
        app.handle_key(
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
            Rect::new(0, 0, 20, 8),
        );
        app.handle_key(
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
            Rect::new(0, 0, 20, 8),
        );
        app.handle_key(
            KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE),
            Rect::new(0, 0, 20, 8),
        );

        let Some(Modal::Text {
            horizontal_scroll, ..
        }) = app.modal
        else {
            panic!("expected text modal");
        };
        assert_eq!(horizontal_scroll, 8);
    }

    #[test]
    fn content_mouse_drag_copies_selected_source_text() {
        let area = Rect::new(0, 0, 40, 10);
        for content_kind in [
            ContentKind::Describe,
            ContentKind::Yaml,
            ContentKind::Events,
        ] {
            let mut app = app();
            app.modal = Some(Modal::Text {
                title: "Content".into(),
                content: "kind: Widget\nmetadata: {}".into(),
                kind: content_kind,
                wrapped: true,
                vertical_scroll: 0,
                horizontal_scroll: 0,
                query: String::new(),
                search_input: None,
                selection: None,
            });
            let body = content_modal_body(area, content_kind);
            let mouse = |kind, column| MouseEvent {
                kind,
                column: body.x + column,
                row: body.y,
                modifiers: KeyModifiers::NONE,
            };

            assert_eq!(
                app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 0), area),
                None
            );
            assert_eq!(
                app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 3), area),
                None
            );
            assert_eq!(
                app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 3), area),
                Some("kind".into())
            );
        }
    }

    #[test]
    fn clipboard_toast_is_rendered_as_a_popup() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = app().theme;
        let toast = Toast {
            message: "● Copied to clipboard".into(),
            expires_at: Instant::now() + TOAST_DURATION,
        };

        terminal
            .draw(|frame| render_toast(frame, frame.area(), &toast, &theme))
            .unwrap();

        assert!(
            terminal
                .backend()
                .to_string()
                .contains("Copied to clipboard")
        );
    }

    #[test]
    fn wrapped_yaml_selection_uses_original_newlines_only() {
        let content = "value: abcdefghijklmnopqrstuvwxyz\nnext: yes";
        let body = Rect::new(0, 0, 10, 10);
        let rows = visual_rows(content, body.width, true);
        let last_row = rows.len() - 1;
        let last_column = rows[last_row]
            .graphemes
            .iter()
            .map(|grapheme| usize::from(grapheme.width.max(1)))
            .sum::<usize>()
            - 1;
        let first = selection_point_at(content, body, 0, 0, 0, 0, true, false).unwrap();
        let last = selection_point_at(
            content,
            body,
            last_column as u16,
            last_row as u16,
            0,
            0,
            true,
            false,
        )
        .unwrap();
        let selection = TextSelection {
            anchor: first,
            focus: last,
        };

        assert_eq!(&content[selection.range()], content);
    }

    #[test]
    fn modal_find_moves_between_matching_lines() {
        let mut scroll = 0;
        move_modal_match(
            "one\nmatch\nthree\nmatch",
            "MATCH",
            &mut scroll,
            false,
            80,
            false,
        );
        assert_eq!(scroll, 1);
        move_modal_match(
            "one\nmatch\nthree\nmatch",
            "MATCH",
            &mut scroll,
            false,
            80,
            false,
        );
        assert_eq!(scroll, 3);
        move_modal_match(
            "one\nmatch\nthree\nmatch",
            "MATCH",
            &mut scroll,
            false,
            80,
            false,
        );
        assert_eq!(scroll, 1);
    }

    #[test]
    fn package_schema_does_not_depend_on_an_image() {
        let snapshot = Snapshot::parse(
            br#"{"object":{"apiVersion":"pkg.crossplane.io/v1","kind":"Provider","metadata":{"name":"provider"}}}"#,
        )
        .unwrap();
        assert!(snapshot.nodes[0].is_package);
        let plan = TablePlan::new(&snapshot, &[0], 100, true, false, false, false);
        let header = plan.header();
        assert!(header.contains("INSTALLED"));
        assert!(header.contains("HEALTHY"));
        assert!(!header.contains("GROUP"));
    }

    #[test]
    fn narrow_package_table_keeps_health_columns_reachable() {
        let snapshot = Snapshot::parse(
            br#"{"object":{"apiVersion":"pkg.crossplane.io/v1","kind":"Provider","metadata":{"name":"provider"}}}"#,
        )
        .unwrap();
        let plan = TablePlan::new(&snapshot, &[0], 30, true, false, false, false);
        let header = plan.header();
        assert!(header.width() <= 30);
        assert!(header.contains("INSTALLED"));
        assert!(header.contains("HEALTHY"));
    }

    #[test]
    fn modal_scroll_is_clamped_to_last_viewport() {
        assert_eq!(modal_max_vertical("one\ntwo\nthree\nfour", 80, 2, false), 2);
        assert_eq!(modal_max_vertical("0123456789", 5, 1, true), 1);
        assert_eq!(modal_max_horizontal("0123456789", 6), 4);
    }

    #[test]
    fn unicode_modal_matches_are_highlighted() {
        let theme = app().theme;
        let line = highlighted_line("CAFÉ", "café", Style::default(), &theme);
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].content, "CAFÉ");
    }

    #[test]
    fn describe_forwards_cluster_selection_to_kubectl() {
        let args = kubectl_args(
            "describe",
            "widgets.example.io/example",
            Some("apps"),
            Some("development"),
            Some(std::path::Path::new("/tmp/kube config")),
        );
        assert_eq!(
            args,
            [
                "describe",
                "widgets.example.io/example",
                "--namespace",
                "apps",
                "--context",
                "development",
                "--kubeconfig",
                "/tmp/kube config",
            ]
            .map(std::ffi::OsString::from)
        );
    }
}
