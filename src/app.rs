// Tree and table presentation portions are adapted from xpdig's
// internal/bubbles/layout/xpnavigator/model.go.
// Copyright 2025 Bruno Luiz da Silva. Licensed under Apache-2.0.
// Translated and substantially modified for xpdelve in 2026. See NOTICE.
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
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
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
    ScrollbarOrientation, ScrollbarState, Wrap,
};
use regex::RegexBuilder;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use tokio::time;
use tokio_util::sync::CancellationToken;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::cli::Cli;
use crate::config::{self, Config};
use crate::kubernetes::{DeletePropagation, Kubernetes, Target};
use crate::model::{HealthFilter, Identity, ProjectedNode, ResourceKind, Snapshot};
use crate::text;
use crate::theme::Theme;
use crate::trace::{self, TraceRequest};

#[derive(Clone, Debug)]
struct Selection {
    visible_index: usize,
    identity: Option<Identity>,
}

impl Selection {
    fn new(visible_index: usize, nodes: &[ProjectedNode], visible: &[usize]) -> Self {
        let identity = visible
            .get(visible_index)
            .and_then(|index| nodes.get(*index))
            .map(|node| node.identity.clone());
        Self {
            visible_index,
            identity,
        }
    }

    fn empty() -> Self {
        Self {
            visible_index: 0,
            identity: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Scroller {
    offset: usize,
    max: usize,
}

impl Scroller {
    fn with_offset(offset: usize, max: usize) -> Self {
        Self {
            offset: offset.min(max),
            max,
        }
    }

    fn scroll_by(mut self, delta: isize) -> Self {
        self.offset = self.offset.saturating_add_signed(delta).min(self.max);
        self
    }

    fn ensure_visible(mut self, item: usize, viewport: usize) -> Self {
        if viewport == 0 {
            return self;
        }
        if item < self.offset {
            self.offset = item;
        } else if item >= self.offset.saturating_add(viewport) {
            self.offset = item.saturating_sub(viewport.saturating_sub(1));
        }
        self.offset = self.offset.min(self.max);
        self
    }

    fn offset(self) -> usize {
        self.offset
    }

    fn offset_u16(self) -> u16 {
        self.offset.try_into().unwrap_or(u16::MAX)
    }
}

const EVENT_BUFFER: usize = 128;

#[derive(Clone, Debug)]
enum ListPickerAction {
    None,
    Close,
    Select(usize),
}

#[derive(Clone, Debug)]
struct ListPicker {
    cursor: usize,
    len: usize,
}

impl ListPicker {
    fn with_cursor(cursor: usize, len: usize) -> Self {
        Self {
            cursor: cursor.min(len.saturating_sub(1)),
            len,
        }
    }

    fn handle_key(&mut self, key: KeyEvent, page_size: usize) -> ListPickerAction {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
            return ListPickerAction::Close;
        }
        if self.len == 0 {
            return ListPickerAction::None;
        }
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                self.cursor = (self.cursor + 1) % self.len;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.cursor = self.cursor.checked_sub(1).unwrap_or(self.len - 1);
            }
            KeyCode::PageDown => {
                self.cursor = self.cursor.saturating_add(page_size).min(self.len - 1);
            }
            KeyCode::PageUp => {
                self.cursor = self.cursor.saturating_sub(page_size);
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.cursor = 0;
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.cursor = self.len - 1;
            }
            KeyCode::Enter => {
                return ListPickerAction::Select(self.cursor);
            }
            _ => {}
        }
        ListPickerAction::None
    }

    fn cursor(&self) -> usize {
        self.cursor
    }
}
const DESCRIBE_OUTPUT_LIMIT: usize = 16 * 1024 * 1024;
const DESCRIBE_ERROR_LIMIT: usize = 1024 * 1024;
const DESCRIBE_TIMEOUT: Duration = Duration::from_secs(60);
const TOAST_DURATION: Duration = Duration::from_millis(1500);
const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(500);
const SELECTION_SCROLL_INTERVAL: Duration = Duration::from_millis(75);
const PALETTE_TITLE: &str = " commands & resources (↑/↓:select, Enter:apply) ";
const HEALTH_FILTERS: [HealthFilter; 3] = [
    HealthFilter::All,
    HealthFilter::Unhealthy,
    HealthFilter::Healthy,
];
const HELP_LINES: &[&str] = &[
    "Navigation",
    "  j/k, Up/Down      move selection",
    "  PgUp/PgDn         move one page",
    "  Mouse wheel       scroll vertically",
    "  Enter             show resource problem details",
    "  Right             expand or select first child",
    "  Left              collapse or select parent",
    "  [                 collapse all below root",
    "  ]                 expand all",
    "  h/l               scroll columns",
    "  L (Shift+L)       toggle logical name column",
    "  X (Shift+X)       toggle external name column",
    "  Shift+Left/Right  scroll columns",
    "  Thumbwheel        scroll columns over tree",
    "  Shift+wheel       scroll columns over tree",
    "",
    "Discovery",
    "  :                 open command palette",
    "  :clear            clear kind filter",
    "  :exclude          exclude resource kinds",
    "  :health           filter by resource health",
    "  :skin             choose theme",
    "  :quit             exit xpdelve",
    "  /                 filter tree",
    "  f                 find text",
    "  n / N             next / previous match",
    "  Esc               clear find, text filter, kind, and health",
    "",
    "Session",
    "  r                 refresh now",
    "  P                 pause automatic refresh",
    "  :reload           reload configuration",
    "  q / Ctrl+C        quit",
    "",
    "Actions",
    "  d / y / s / E     describe / live YAML / status / events",
    "  e                 kubectl edit",
    "  c                 copy resource identifier",
    "  p / u             pause / unpause resource",
    "  Ctrl+D            delete resource",
    "  Ctrl+X            select finalizers to remove",
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
    Command,
    Filter,
    Find,
    Help,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PaletteAction {
    FilterKind(ResourceKind),
    ClearKindFilter,
    OpenExcludePicker,
    OpenHealthPicker,
    OpenSkinPicker,
    ReloadConfig,
    Quit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PaletteEntry {
    label: String,
    action: PaletteAction,
}

#[derive(Clone, Debug)]
struct TextModalState {
    wrapped: bool,
    vertical_scroll: u16,
    horizontal_scroll: u16,
    query: String,
    search_input: Option<String>,
}

impl TextModalState {
    fn new(kind: ContentKind) -> Self {
        Self {
            wrapped: kind.wraps_by_default(),
            vertical_scroll: 0,
            horizontal_scroll: 0,
            query: String::new(),
            search_input: None,
        }
    }

    fn handle_key(
        &mut self,
        key: KeyEvent,
        content: &content::TextContent,
        kind: ContentKind,
        body_size: (usize, usize),
        page_size: usize,
    ) -> TextModalAction {
        let (body_width, body_height) = body_size;
        let (max_vertical, max_horizontal) =
            content.scroll_bounds(body_width as u16, body_height as u16, self.wrapped);

        if let Some(input) = &mut self.search_input {
            match key.code {
                KeyCode::Esc => {
                    self.search_input = None;
                }
                KeyCode::Enter => {
                    self.query = input.trim().to_owned();
                    self.search_input = None;
                    return TextModalAction::FindNext;
                }
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char(character) => input.push(character),
                _ => {}
            }
            return TextModalAction::None;
        }

        match (key.code, key.modifiers) {
            (KeyCode::Esc | KeyCode::Char('q'), _) => TextModalAction::Close,
            (KeyCode::Down | KeyCode::Char('j'), _) => {
                self.vertical_scroll = self.vertical_scroll.saturating_add(1).min(max_vertical);
                TextModalAction::None
            }
            (KeyCode::Up | KeyCode::Char('k'), _) => {
                self.vertical_scroll = self.vertical_scroll.saturating_sub(1);
                TextModalAction::None
            }
            (KeyCode::PageDown, _) => {
                self.vertical_scroll = self
                    .vertical_scroll
                    .saturating_add(body_height.try_into().unwrap_or(u16::MAX))
                    .min(max_vertical);
                TextModalAction::None
            }
            (KeyCode::PageUp, _) => {
                self.vertical_scroll = self
                    .vertical_scroll
                    .saturating_sub(page_size.try_into().unwrap_or(u16::MAX));
                TextModalAction::None
            }
            (KeyCode::Home | KeyCode::Char('g'), _) => {
                self.vertical_scroll = 0;
                TextModalAction::None
            }
            (KeyCode::End | KeyCode::Char('G'), _) => {
                self.vertical_scroll = max_vertical;
                TextModalAction::None
            }
            (KeyCode::Left, _) | (KeyCode::Char('h'), KeyModifiers::NONE) => {
                self.horizontal_scroll = self.horizontal_scroll.saturating_sub(4);
                TextModalAction::None
            }
            (KeyCode::Right, _) | (KeyCode::Char('l'), KeyModifiers::NONE) => {
                self.horizontal_scroll =
                    self.horizontal_scroll.saturating_add(4).min(max_horizontal);
                TextModalAction::None
            }
            (KeyCode::Char('w'), KeyModifiers::NONE) if kind == ContentKind::Yaml => {
                self.wrapped = !self.wrapped;
                self.vertical_scroll = 0;
                self.horizontal_scroll = 0;
                TextModalAction::None
            }
            (KeyCode::Char('/'), _) => {
                self.search_input = Some(self.query.clone());
                TextModalAction::None
            }
            (KeyCode::Char('n'), _) => TextModalAction::FindNext,
            (KeyCode::Char('N'), _) => TextModalAction::FindPrevious,
            _ => TextModalAction::None,
        }
    }
}

#[derive(Clone, Debug)]
enum TextModalAction {
    None,
    Close,
    FindNext,
    FindPrevious,
}

#[derive(Clone, Debug)]
enum Modal {
    Text {
        title: String,
        content: content::TextContent,
        kind: ContentKind,
        state: TextModalState,
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
    ContextMenu {
        target: Target,
        column: u16,
        row: u16,
        pressed: Option<usize>,
    },
    SkinPicker {
        selected: usize,
    },
    HealthPicker {
        selected: usize,
    },
    ExcludePicker {
        kinds: Vec<ResourceKind>,
        excluded: HashSet<ResourceKind>,
        cursor: usize,
        scroll: usize,
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
    dragged: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TreeSelection {
    text: TextSelection,
    content: String,
    area: Rect,
    selecting: bool,
}

#[derive(Clone, Debug)]
struct TreeClick {
    identity: Identity,
    at: Instant,
}

#[derive(Clone, Copy, Debug)]
struct TextDrag {
    document: u64,
    mouse: MouseEvent,
    next_scroll: Instant,
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
    Error,
    SmallError,
}

impl ContentKind {
    fn supports_mouse_selection(self) -> bool {
        matches!(
            self,
            Self::Describe | Self::Yaml | Self::Events | Self::Error | Self::SmallError
        )
    }

    fn wraps_by_default(self) -> bool {
        matches!(
            self,
            Self::Describe | Self::Yaml | Self::Events | Self::Error | Self::SmallError
        )
    }
}

struct App {
    resource: String,
    snapshot: Option<Arc<Snapshot>>,
    selection: Selection,
    resource_scroll: usize,
    resource_horizontal_scroll: usize,
    show_logical_name: bool,
    show_external_name: bool,
    collapsed: HashSet<Identity>,
    excluded_kinds: HashSet<ResourceKind>,
    mode: InputMode,
    help_scroll: u16,
    input: String,
    filter: String,
    kind_filter: Option<ResourceKind>,
    health_filter: HealthFilter,
    palette_selected: usize,
    find: String,
    loading: bool,
    paused: bool,
    generation: u64,
    last_refresh: Option<Instant>,
    status: String,
    quit: bool,
    config: Config,
    config_path: PathBuf,
    cli: Cli,
    no_watch: bool,
    theme: Theme,
    kubernetes: Option<Arc<Kubernetes>>,
    kubernetes_status: String,
    modal: Option<Modal>,
    active_mutations: HashSet<Identity>,
    refresh_pending: bool,
    retry_delay: Duration,
    resource_missing: bool,
    namespace: Option<String>,
    context: Option<String>,
    toast: Option<Toast>,
    tree_selection: Option<TreeSelection>,
    text_drag: Option<TextDrag>,
    deferred_snapshot: Option<Snapshot>,
    last_tree_click: Option<TreeClick>,
}

mod content;
mod events;
mod input;
mod render;
mod runtime;
mod selection;
mod state;
mod terminal;

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

#[derive(Debug)]
enum MouseAction {
    Copy(String),
    Action(UiAction),
}

pub use runtime::run;

#[cfg(test)]
#[path = "app/tests.rs"]
mod tests;
