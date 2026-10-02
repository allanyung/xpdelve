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

const EVENT_BUFFER: usize = 128;
const DESCRIBE_OUTPUT_LIMIT: usize = 16 * 1024 * 1024;
const DESCRIBE_ERROR_LIMIT: usize = 1024 * 1024;
const DESCRIBE_TIMEOUT: Duration = Duration::from_secs(60);
const TOAST_DURATION: Duration = Duration::from_millis(1500);
const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(500);
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
    "  Enter             show resource problem details",
    "  Right             expand or select first child",
    "  Left              collapse or select parent",
    "  [                 collapse all below root",
    "  ]                 expand all",
    "  h/l               scroll columns",
    "  Shift+Left/Right  scroll columns",
    "  Thumbwheel        scroll columns over tree",
    "  Shift+wheel       scroll columns over tree",
    "",
    "Discovery",
    "  :                 open command palette",
    "  :exclude          exclude resource kinds",
    "  :health           filter by resource health",
    "  /                 filter tree",
    "  f                 find text",
    "  n / N             next / previous match",
    "  Esc               clear find, text filter, and kind",
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
}

struct App {
    resource: String,
    snapshot: Option<Arc<Snapshot>>,
    selected_identity: Option<Identity>,
    selected_visible: usize,
    resource_scroll: usize,
    resource_horizontal_scroll: usize,
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
    deferred_snapshot: Option<Snapshot>,
    last_tree_click: Option<TreeClick>,
}

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
