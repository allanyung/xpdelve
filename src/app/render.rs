use super::selection::*;
use super::*;
use crate::columns::{self, ExtraColumn};

pub(super) fn render(frame: &mut ratatui::Frame<'_>, app: &App) {
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
    if app.mode == InputMode::Command {
        render_palette(frame, area, app);
    }
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
            Modal::ContextMenu { column, row, .. } => context_menu_area(area, *column, *row),
            Modal::SkinPicker { .. } => centered(
                area,
                58,
                u16::try_from(crate::theme::BUILTIN_NAMES.len())
                    .unwrap_or(u16::MAX)
                    .saturating_add(2),
            ),
            Modal::HealthPicker { .. } => centered(area, 58, 5),
            Modal::ExcludePicker { kinds, .. } => exclude_picker_area(area, kinds.len()),
        };
        render_modal(frame, modal_area, modal, &app.theme);
    }
    if let Some(toast) = &app.toast
        && toast.expires_at > Instant::now()
    {
        render_toast(frame, area, toast, &app.theme);
    }
}

pub(super) fn resource_tree_area(area: Rect) -> Option<Rect> {
    if area.width < 32 || area.height < 8 {
        return None;
    }
    Some(
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(3),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(area)[1],
    )
}

pub(super) fn render_toast(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    toast: &Toast,
    theme: &Theme,
) {
    let width = u16::try_from(toast.message.width())
        .unwrap_or(u16::MAX)
        .saturating_add(4)
        .min(area.width.saturating_sub(2));
    let height = 3.min(area.height);
    let popup = Rect::new(
        area.x.saturating_add(area.width.saturating_sub(width) / 2),
        area.y
            .saturating_add(area.height.saturating_sub(height).saturating_sub(3)),
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

pub(super) fn render_header(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
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

pub(super) fn render_tree(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
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
    let Some(rendered) = rendered_tree(app, area) else {
        return;
    };
    let mut lines = Vec::with_capacity(rendered.row_count + 1);
    lines.push(Line::styled(
        rendered.lines[0].clone(),
        Style::default().add_modifier(Modifier::BOLD),
    ));
    for (row, (visible_position, index)) in visible
        .iter()
        .enumerate()
        .skip(rendered.start)
        .take(rendered.row_count)
        .enumerate()
    {
        let node = &snapshot.nodes[*index];
        let content = rendered.lines[row + 1].clone();
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
    if visible.is_empty() {
        let message = if app.health_filter == HealthFilter::All {
            "No resources match the active filters".to_owned()
        } else {
            format!(
                "No resources match Health: {}",
                health_filter_label(app.health_filter)
            )
        };
        lines.push(Line::styled(message, app.theme.subtle()));
    }
    let title = format!(" Resources ({}/{}) ", visible.len(), snapshot.nodes.len());
    frame.render_widget(
        Paragraph::new(lines).block(bordered_block(title, &app.theme)),
        area,
    );
    let horizontal = rendered.horizontal;
    let body = Block::default().borders(Borders::ALL).inner(area);
    for divider in [
        horizontal.pinned_width.checked_sub(2),
        (horizontal.right_pinned_width >= 2)
            .then_some(horizontal.pinned_width + horizontal.viewport_width),
    ]
    .into_iter()
    .flatten()
    {
        let divider_column = body.x + divider as u16;
        for row in 0..=rendered.row_count {
            if let Some(cell) = frame
                .buffer_mut()
                .cell_mut((divider_column, body.y + row as u16))
            {
                cell.set_style(app.theme.subtle());
            }
        }
    }
    if horizontal.max_offset() > 0 && horizontal.viewport_width > 0 {
        let scrollbar_area = Rect::new(
            area.x + horizontal.pinned_width as u16,
            area.y + area.height.saturating_sub(1),
            horizontal.viewport_width as u16 + 1,
            1,
        );
        let mut state = ScrollbarState::new(horizontal.max_offset() + 1)
            .position(horizontal.offset)
            .viewport_content_length(horizontal.viewport_width);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::HorizontalBottom)
                .begin_symbol(Some(if app.config.ui.ascii { "<" } else { "◀" }))
                .end_symbol(Some(if app.config.ui.ascii { ">" } else { "▶" }))
                .begin_style(if horizontal.offset == 0 {
                    app.theme.subtle().add_modifier(Modifier::DIM)
                } else {
                    app.theme.title()
                })
                .end_style(if horizontal.offset == horizontal.max_offset() {
                    app.theme.subtle().add_modifier(Modifier::DIM)
                } else {
                    app.theme.title()
                })
                .track_symbol(Some(if app.config.ui.ascii { "." } else { "·" }))
                .track_style(app.theme.subtle())
                .thumb_symbol(if app.config.ui.ascii { "-" } else { "▪" })
                .thumb_style(if app.theme.colors_enabled {
                    app.theme.fg(app.theme.palette.subtext1)
                } else {
                    app.theme.subtle()
                }),
            scrollbar_area,
            &mut state,
        );
    }
    if let Some(selection) = &app.tree_selection {
        render_text_selection(
            frame,
            body,
            &rendered.content,
            selection.text,
            0,
            0,
            false,
            &app.theme,
        );
    }
}

pub(super) struct RenderedTree {
    pub(super) lines: Vec<String>,
    pub(super) content: String,
    pub(super) start: usize,
    pub(super) row_count: usize,
    pub(super) horizontal: TreeHorizontalLayout,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct TreeHorizontalLayout {
    // Pinned regions include their two-cell divider; STATUS also has one trailing space.
    pub(super) pinned_width: usize,
    pub(super) right_pinned_width: usize,
    pub(super) viewport_width: usize,
    pub(super) content_width: usize,
    pub(super) offset: usize,
}

impl TreeHorizontalLayout {
    pub(super) fn max_offset(self) -> usize {
        if self.viewport_width == 0 {
            0
        } else {
            self.content_width.saturating_sub(self.viewport_width)
        }
    }

    fn slice(self, value: &str, ascii: bool) -> String {
        let object_width = self.pinned_width.saturating_sub(2);
        let object = horizontal_slice(value, 0, object_width);
        let separator = horizontal_slice(
            if ascii { "| " } else { "│ " },
            0,
            self.pinned_width - object_width,
        );
        let mut middle = horizontal_slice(
            value,
            self.pinned_width + self.offset,
            self.viewport_width
                .min(self.content_width.saturating_sub(self.offset)),
        );
        middle.push_str(&" ".repeat(self.viewport_width.saturating_sub(middle.width())));
        let right_separator = horizontal_slice(
            if ascii { "| " } else { "│ " },
            0,
            self.right_pinned_width.min(2),
        );
        let status = horizontal_slice(
            value,
            self.pinned_width + self.content_width + 2,
            self.right_pinned_width.saturating_sub(3),
        );
        let padding = if self.right_pinned_width >= 3 {
            " "
        } else {
            ""
        };
        format!("{object}{separator}{middle}{right_separator}{status}{padding}")
    }
}

pub(super) fn tree_table_plan(app: &App, area: Rect, visible: &[usize]) -> Option<TablePlan> {
    let snapshot = app.snapshot.as_ref()?;
    Some(
        TablePlan::new(
            snapshot,
            visible,
            area.width.saturating_sub(2) as usize,
            snapshot.nodes.first().is_some_and(|node| node.is_package),
            app.config.ui.short,
            app.config.ui.ascii,
        )
        .with_extra_columns(columns::resolve(
            &app.config.extra_columns,
            snapshot,
            visible,
        )),
    )
}

pub(super) fn rendered_tree(app: &App, area: Rect) -> Option<RenderedTree> {
    let snapshot = app.snapshot.as_ref()?;
    let visible = app.visible();
    let viewport = area.height.saturating_sub(3) as usize;
    let start = app.resource_view_start(viewport);
    let plan = tree_table_plan(app, area, &visible)?;
    let horizontal = plan.horizontal_layout(app.resource_horizontal_scroll);
    let mut lines = Vec::with_capacity(viewport + 1);
    lines.push(horizontal.slice(&plan.header(), app.config.ui.ascii));
    lines.extend(visible.iter().skip(start).take(viewport).map(|index| {
        let node = &snapshot.nodes[*index];
        horizontal.slice(
            &plan.row(
                snapshot,
                node,
                app.kind_filter.is_none() || app.health_filter != HealthFilter::All,
                app.collapsed.contains(&node.identity),
            ),
            app.config.ui.ascii,
        )
    }));
    let row_count = lines.len().saturating_sub(1);
    let content = lines.join("\n");
    Some(RenderedTree {
        lines,
        content,
        start,
        row_count,
        horizontal,
    })
}

pub(super) fn clicked_tree_position(
    body: Rect,
    column: u16,
    row: u16,
    start: usize,
    row_count: usize,
) -> Option<usize> {
    let inside = column >= body.x
        && column < body.x.saturating_add(body.width)
        && row > body.y
        && row < body.y.saturating_add(body.height);
    let row = inside.then(|| usize::from(row - body.y - 1))?;
    (row < row_count).then_some(start + row)
}

pub(super) fn fuzzy_score(candidate: &str, query: &str) -> Option<usize> {
    let candidate = candidate.to_lowercase();
    let query = query.to_lowercase();
    if query.is_empty() {
        return Some(0);
    }
    if candidate == query {
        return Some(10_000);
    }
    if candidate.starts_with(&query) {
        return Some(5_000usize.saturating_sub(candidate.len()));
    }

    let mut score = 0usize;
    let mut previous = None;
    let mut candidates = candidate.char_indices();
    for wanted in query.chars() {
        let (index, _) = candidates.find(|(_, character)| *character == wanted)?;
        score += 10;
        if previous.is_some_and(|previous| index == previous + 1) {
            score += 5;
        }
        previous = Some(index);
    }
    Some(score.saturating_sub(candidate.len()))
}

pub(super) fn resource_kind_label(kind: &ResourceKind, qualified: bool) -> String {
    if !qualified {
        return kind.kind.clone();
    }
    let group = if kind.group.is_empty() {
        "core"
    } else {
        &kind.group
    };
    format!("{}.{group}", kind.kind)
}

pub(super) fn health_filter_label(filter: HealthFilter) -> &'static str {
    match filter {
        HealthFilter::All => "All",
        HealthFilter::Unhealthy => "Unhealthy",
        HealthFilter::Healthy => "Healthy",
    }
}

pub(super) fn render_resource_missing(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
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

pub(super) fn condition_text(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "True",
        Some(false) => "False",
        None => "-",
    }
}

pub(super) struct TablePlan {
    available: usize,
    object: usize,
    group: usize,
    status: usize,
    package: bool,
    timestamps: bool,
    version: usize,
    state: usize,
    ascii: bool,
    extra_columns: Vec<ExtraColumn>,
}

impl TablePlan {
    pub(super) fn new(
        snapshot: &Snapshot,
        visible: &[usize],
        available: usize,
        package: bool,
        short: bool,
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
        let version = if package {
            visible
                .iter()
                .filter_map(|index| snapshot.nodes[*index].version.as_deref())
                .map(UnicodeWidthStr::width)
                .max()
                .unwrap_or_default()
                .max("VERSION".len())
        } else {
            0
        };
        let state = if package {
            visible
                .iter()
                .filter_map(|index| snapshot.nodes[*index].state.as_deref())
                .map(UnicodeWidthStr::width)
                .max()
                .unwrap_or_default()
                .max("STATE".len())
        } else {
            0
        };
        let status = visible
            .iter()
            .map(|index| status_cell(&snapshot.nodes[*index]).width())
            .max()
            .unwrap_or(6)
            .max("STATUS".len())
            .min(24)
            .min((available / 3).max("STATUS".len()))
            .min(available.saturating_sub(5));
        let object = object.min(available.saturating_sub(status + 5) * 3 / 5);
        Self {
            available,
            object,
            group,
            status,
            package,
            timestamps,
            version,
            state,
            ascii,
            extra_columns: Vec::new(),
        }
    }

    fn with_extra_columns(mut self, columns: Vec<ExtraColumn>) -> Self {
        self.extra_columns = columns;
        self
    }

    pub(super) fn fixed_width(&self) -> usize {
        if self.package {
            let displayed = 3
                + usize::from(self.version > 0)
                + usize::from(self.state > 0)
                + usize::from(self.timestamps) * 2;
            let widths = 9 + 7 + if self.timestamps { 30 } else { 0 } + self.version + self.state;
            widths + displayed * 2
        } else if self.timestamps {
            53
        } else {
            19
        }
    }

    pub(super) fn horizontal_layout(&self, offset: usize) -> TreeHorizontalLayout {
        let right_pinned_width = (self.status + 3).min(self.available);
        let pinned_width = (self.object + 2).min(self.available.saturating_sub(right_pinned_width));
        let viewport_width = self
            .available
            .saturating_sub(pinned_width + right_pinned_width);
        let group = if self.package { 0 } else { self.group };
        let extra_width: usize = self
            .extra_columns
            .iter()
            .map(|column| column.width + 2)
            .sum();
        let content_width = (self.object + group + self.fixed_width() + extra_width + self.status)
            .saturating_sub(pinned_width + self.status + 2);
        let mut horizontal = TreeHorizontalLayout {
            pinned_width,
            right_pinned_width,
            viewport_width,
            content_width,
            offset: 0,
        };
        horizontal.offset = offset.min(horizontal.max_offset());
        horizontal
    }

    pub(super) fn header(&self) -> String {
        let mut columns = if self.package {
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
            columns
        } else if self.timestamps {
            vec![
                ("OBJECT", self.object),
                ("GROUP", self.group),
                ("SYNCED", 6),
                ("SYNCED LAST", 15),
                ("READY", 5),
                ("READY LAST", 15),
            ]
        } else {
            vec![
                ("OBJECT", self.object),
                ("GROUP", self.group),
                ("SYNCED", 6),
                ("READY", 5),
            ]
        };
        columns.extend(
            self.extra_columns
                .iter()
                .map(|column| (column.header.as_str(), column.width)),
        );
        columns.push(("STATUS", self.status));
        format_columns(&columns)
    }

    pub(super) fn row(
        &self,
        snapshot: &Snapshot,
        node: &ProjectedNode,
        show_tree_prefix: bool,
        collapsed: bool,
    ) -> String {
        let object =
            object_cell_with_tree_state(snapshot, node, self.ascii, show_tree_prefix, collapsed);
        let object = compact_object(&object, self.object);
        let group = group_cell(node);
        let version = text::sanitize(node.version.as_deref().unwrap_or("-"));
        let state = text::sanitize(node.state.as_deref().unwrap_or("-"));
        let status = status_cell(node);
        let extra_values: Vec<_> = self
            .extra_columns
            .iter()
            .map(|column| column.value(node))
            .collect();
        let mut columns = if self.package {
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
            columns
        } else if self.timestamps {
            vec![
                (object.as_str(), self.object),
                (group.as_str(), self.group),
                (condition_text(node.synced), 6),
                (node.synced_last.as_deref().unwrap_or("-"), 15),
                (condition_text(node.ready), 5),
                (node.ready_last.as_deref().unwrap_or("-"), 15),
            ]
        } else {
            vec![
                (object.as_str(), self.object),
                (group.as_str(), self.group),
                (condition_text(node.synced), 6),
                (condition_text(node.ready), 5),
            ]
        };
        columns.extend(
            self.extra_columns
                .iter()
                .zip(&extra_values)
                .map(|(column, value)| (value.as_str(), column.width)),
        );
        columns.push((status.as_str(), self.status));
        format_columns_owned(&columns)
    }
}

pub(super) fn object_cell(snapshot: &Snapshot, node: &ProjectedNode, ascii: bool) -> String {
    object_cell_with_state(snapshot, node, ascii)
}

pub(super) fn object_cell_with_state(
    snapshot: &Snapshot,
    node: &ProjectedNode,
    ascii: bool,
) -> String {
    object_cell_with_tree_state(snapshot, node, ascii, true, false)
}

pub(super) fn object_cell_with_tree_state(
    snapshot: &Snapshot,
    node: &ProjectedNode,
    ascii: bool,
    show_tree_prefix: bool,
    collapsed: bool,
) -> String {
    let mut prefix = if show_tree_prefix {
        tree_prefix(snapshot, node, ascii)
    } else {
        String::new()
    };
    let leaf_with_tree_prefix = node.child_count == 0 && !prefix.is_empty();
    if leaf_with_tree_prefix {
        prefix.pop();
    }
    let disclosure = match (node.child_count > 0, collapsed, ascii) {
        (false, _, true) if leaf_with_tree_prefix => "-- ",
        (false, _, false) if leaf_with_tree_prefix => "── ",
        (false, _, _) => "  ",
        (true, true, true) => "+ ",
        (true, false, true) => "- ",
        (true, true, false) => "▸ ",
        (true, false, false) => "▾ ",
    };
    let paused = if node.paused { " (paused)" } else { "" };
    format!(
        "{prefix}{disclosure}{}/{}{paused}",
        text::sanitize(&node.identity.kind),
        text::sanitize(&node.identity.name)
    )
}

pub(super) fn group_cell(node: &ProjectedNode) -> String {
    if node.identity.group.is_empty() {
        "core".into()
    } else {
        text::sanitize(&node.identity.group)
    }
}

fn status_cell(node: &ProjectedNode) -> String {
    text::sanitize(&node.status).replace(['\n', '\t'], " ")
}

pub(super) fn format_columns(columns: &[(&str, usize)]) -> String {
    format_columns_owned(columns)
}

pub(super) fn format_columns_owned(columns: &[(&str, usize)]) -> String {
    columns
        .iter()
        .map(|(value, width)| pad_or_truncate(value, *width))
        .collect::<Vec<_>>()
        .join("  ")
}

pub(super) fn pad_or_truncate(value: &str, width: usize) -> String {
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
    for grapheme in value.graphemes(true) {
        let grapheme_width = grapheme.width();
        if used + grapheme_width > target {
            break;
        }
        result.push_str(grapheme);
        used += grapheme_width;
    }
    result.push('…');
    result.push_str(&" ".repeat(width.saturating_sub(result.width())));
    result
}

pub(super) fn compact_object(value: &str, width: usize) -> String {
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

pub(super) fn horizontal_slice(value: &str, offset: usize, width: usize) -> String {
    let mut position = 0;
    let end = offset.saturating_add(width);
    let mut result = String::new();
    for grapheme in value.graphemes(true) {
        let next = position + grapheme.width();
        if next <= offset {
            position = next;
            continue;
        }
        if position >= end {
            break;
        }
        if position < offset || next > end {
            // A wide grapheme crossing an edge occupies blank cells rather than
            // shifting every subsequent column or drawing half a glyph.
            result.push_str(&" ".repeat(next.min(end).saturating_sub(position.max(offset))));
        } else {
            result.push_str(grapheme);
        }
        position = next;
    }
    result
}

pub(super) fn tree_prefix(snapshot: &Snapshot, node: &ProjectedNode, ascii: bool) -> String {
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

pub(super) fn render_prompt(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let line = if app.resource_missing {
        " r:retry  q:quit".into()
    } else {
        match app.mode {
            InputMode::Command => format!(":{}▏", app.input),
            InputMode::Filter => format!(" Filter: {}_", app.input),
            InputMode::Find => format!(" Find: {}_", app.input),
            InputMode::Normal | InputMode::Help
                if app.kind_filter.is_some()
                    || app.health_filter != HealthFilter::All
                    || !app.excluded_kinds.is_empty()
                    || !app.filter.is_empty()
                    || !app.find.is_empty() =>
            {
                let mut active = Vec::new();
                if let Some(kind) = &app.kind_filter {
                    active.push(format!("Kind: {}", app.kind_filter_label(kind)));
                }
                if app.health_filter != HealthFilter::All {
                    active.push(format!(
                        "Health: {}",
                        health_filter_label(app.health_filter)
                    ));
                }
                if !app.excluded_kinds.is_empty() {
                    let mut excluded = app
                        .excluded_kinds
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>();
                    excluded.sort_by_key(|kind| kind.to_lowercase());
                    active.push(if excluded.len() <= 2 {
                        format!("Excluded: {}", excluded.join(", "))
                    } else {
                        format!("Excluded kinds: {}", excluded.len())
                    });
                }
                if !app.filter.is_empty() {
                    active.push(format!("Filter: {}", app.filter));
                }
                if !app.find.is_empty() {
                    active.push(format!("Find: {}", app.find));
                }
                format!(" {}", active.join("  |  "))
            }
            InputMode::Normal | InputMode::Help => {
                let details = if app
                    .selected_node()
                    .is_some_and(|node| node.status_details.is_some())
                {
                    " Enter:details "
                } else {
                    ""
                };
                format!(
                    "{details} Left/Right:collapse/expand  ::command  ?:help  /:filter  ctrl-d:delete  d:describe  y:YAML  e:edit  s:status  E:events"
                )
            }
        }
    };
    let scrollable = !app.resource_missing
        && matches!(app.mode, InputMode::Normal | InputMode::Help)
        && resource_tree_area(frame.area())
            .and_then(|tree| tree_table_plan(app, tree, &app.visible()))
            .is_some_and(|plan| {
                let horizontal = plan.horizontal_layout(app.resource_horizontal_scroll);
                horizontal.max_offset() > 0 && horizontal.viewport_width > 0
            });
    if scrollable {
        let chunks = Layout::horizontal([Constraint::Min(0), Constraint::Length(12)]).split(area);
        frame.render_widget(Paragraph::new(line).style(app.theme.subtle()), chunks[0]);
        frame.render_widget(
            Paragraph::new(" h/l:scroll ")
                .alignment(Alignment::Right)
                .style(app.theme.subtle()),
            chunks[1],
        );
    } else {
        frame.render_widget(Paragraph::new(line).style(app.theme.subtle()), area);
    }
}

pub(super) fn render_palette(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let entries = app.palette_entries();
    let popup = palette_area(area, entries.len());
    frame.render_widget(Clear, popup);
    let items = if entries.is_empty() {
        vec![ListItem::new("  No matching resource kinds")]
    } else {
        entries
            .iter()
            .map(|entry| palette_list_item(entry, &app.theme))
            .collect()
    };
    let mut state = ListState::default();
    if !entries.is_empty() {
        state.select(Some(app.palette_selected.min(entries.len() - 1)));
    }
    let list = List::new(items)
        .block(bordered_block(PALETTE_TITLE, &app.theme))
        .highlight_style(app.theme.selected_row());
    frame.render_stateful_widget(list, popup, &mut state);
}

pub(super) fn palette_list_item<'a>(entry: &'a PaletteEntry, theme: &Theme) -> ListItem<'a> {
    match &entry.action {
        PaletteAction::FilterKind(_) => ListItem::new(format!("  {}", entry.label)),
        PaletteAction::ClearKindFilter
        | PaletteAction::OpenExcludePicker
        | PaletteAction::OpenHealthPicker
        | PaletteAction::OpenSkinPicker
        | PaletteAction::ReloadConfig
        | PaletteAction::Quit => ListItem::new(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!(":{}", entry.label),
                Style::default().fg(theme.palette.peach),
            ),
            Span::styled("  cmd", theme.subtle()),
        ])),
    }
}

pub(super) fn palette_area(area: Rect, entry_count: usize) -> Rect {
    let minimum_width = u16::try_from(PALETTE_TITLE.width())
        .unwrap_or(u16::MAX)
        .saturating_add(2);
    let width = 48.max(minimum_width).min(area.width.saturating_sub(2));
    let item_count = entry_count.clamp(1, 12);
    let height = u16::try_from(item_count)
        .unwrap_or(u16::MAX)
        .saturating_add(2)
        .min(area.height.saturating_sub(3));
    Rect::new(
        area.x.saturating_add(1),
        area.y
            .saturating_add(area.height.saturating_sub(height).saturating_sub(3)),
        width,
        height,
    )
}

pub(super) fn exclude_picker_area(area: Rect, kind_count: usize) -> Rect {
    centered(
        area,
        80,
        u16::try_from(kind_count)
            .unwrap_or(u16::MAX)
            .saturating_add(3)
            .clamp(8, 24),
    )
}

pub(super) fn render_status(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
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

pub(super) fn render_help(frame: &mut ratatui::Frame<'_>, area: Rect, scroll: u16, theme: &Theme) {
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
        Paragraph::new("j/k:scroll  PgUp/PgDn:page  g/G:top/bottom  Esc/q/?:close")
            .style(theme.subtle()),
        regions[1],
    );
}

pub(super) fn render_modal(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    modal: &Modal,
    theme: &Theme,
) {
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
            let title = format!(" {} ", text::sanitize(title));
            let block = if matches!(kind, ContentKind::Error | ContentKind::SmallError) {
                destructive_block(title, theme)
            } else {
                bordered_block(title, theme)
            };
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
                    Span::styled(":Change propagation   ", subtle_style),
                    Span::styled("Enter:Delete", delete_style),
                    Span::styled("   Esc:Cancel ", subtle_style),
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
                    Span::styled(":Select   ", subtle_style),
                    Span::styled("Space", selected_style),
                    Span::styled(":Toggle   ", subtle_style),
                    Span::styled("Enter:Remove", remove_style),
                    Span::styled("   Esc:Cancel ", subtle_style),
                ])),
                regions[1],
            );
        }
        Modal::ContextMenu { pressed, .. } => {
            let block = bordered_block(" Resource ", theme);
            let inner = block.inner(area);
            frame.render_widget(block, area);
            frame.render_widget(
                Paragraph::new(vec![
                    Line::styled(" YAML", context_menu_item_style(theme, 0, *pressed)),
                    Line::styled(" Edit", context_menu_item_style(theme, 1, *pressed)),
                    Line::styled(" Status", context_menu_item_style(theme, 2, *pressed)),
                    Line::styled(" Events", context_menu_item_style(theme, 3, *pressed)),
                    Line::styled(" Describe", context_menu_item_style(theme, 4, *pressed)),
                ]),
                inner,
            );
        }
        Modal::SkinPicker { selected } => {
            let mut state = ListState::default().with_selected(Some(*selected));
            let items = crate::theme::BUILTIN_NAMES
                .iter()
                .map(|name| ListItem::new(format!("  {name}")))
                .collect::<Vec<_>>();
            let list = List::new(items)
                .block(bordered_block(
                    " Skins (↑/↓:select, Enter:apply, Esc:cancel) ",
                    theme,
                ))
                .highlight_style(theme.selected_row());
            frame.render_stateful_widget(list, area, &mut state);
        }
        Modal::HealthPicker { selected } => {
            let mut state = ListState::default().with_selected(Some(*selected));
            let items = HEALTH_FILTERS
                .iter()
                .map(|filter| ListItem::new(format!("  {}", health_filter_label(*filter))))
                .collect::<Vec<_>>();
            let list = List::new(items)
                .block(bordered_block(
                    " Health filter (↑/↓:select, Enter:apply, Esc:cancel) ",
                    theme,
                ))
                .highlight_symbol("● ")
                .highlight_style(theme.selected_row());
            frame.render_stateful_widget(list, area, &mut state);
        }
        Modal::ExcludePicker {
            kinds,
            excluded,
            cursor,
            scroll,
        } => {
            let block = bordered_block(" Exclude resource kinds ", theme);
            let inner = block.inner(area);
            frame.render_widget(Clear, area);
            frame.render_widget(block, area);
            let regions =
                Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);
            let visible_count = usize::from(regions[0].height);
            let end = scroll.saturating_add(visible_count).min(kinds.len());
            let items = kinds[*scroll..end]
                .iter()
                .map(|kind| {
                    let hidden = excluded.contains(kind);
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            if hidden { " [x] " } else { " [ ] " },
                            if hidden {
                                theme.danger()
                            } else {
                                theme.subtle()
                            },
                        ),
                        Span::raw(kind.to_string()),
                    ]))
                })
                .collect::<Vec<_>>();
            let mut state = ListState::default();
            if !kinds.is_empty() {
                state.select(Some(cursor.saturating_sub(*scroll)));
            }
            frame.render_stateful_widget(
                List::new(items).highlight_style(theme.selected_row()),
                regions[0],
                &mut state,
            );
            frame.render_widget(
                Paragraph::new(
                    " Space:toggle  a:show all  x:hide all  o:show only  Enter:apply  Esc:cancel",
                )
                .style(theme.subtle()),
                regions[1],
            );
        }
    }
}

pub(super) fn context_menu_item_style(theme: &Theme, item: usize, pressed: Option<usize>) -> Style {
    if pressed == Some(item) {
        theme.selected_option()
    } else {
        theme.fg(theme.palette.text)
    }
}

pub(super) fn target_summary(target: &Target, width: usize) -> (String, String) {
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

pub(super) fn styled_content_line<'a>(
    line: &'a str,
    kind: ContentKind,
    query: &str,
    theme: &Theme,
) -> Line<'a> {
    let base = match kind {
        ContentKind::Events if line.trim_start().starts_with("Warning") => {
            theme.fg(theme.palette.red).bold()
        }
        ContentKind::Describe if line.ends_with(':') => theme.syntax_heading(),
        ContentKind::Yaml => return yaml_line(line, query, theme),
        ContentKind::Describe
        | ContentKind::Events
        | ContentKind::Error
        | ContentKind::SmallError => Style::default(),
    };
    highlighted_line(line, query, base, theme)
}

pub(super) fn content_wraps_by_default(kind: ContentKind) -> bool {
    matches!(
        kind,
        ContentKind::Describe
            | ContentKind::Yaml
            | ContentKind::Events
            | ContentKind::Error
            | ContentKind::SmallError
    )
}

pub(super) fn content_modal_footer(kind: ContentKind, wrapped: bool) -> &'static str {
    match (kind, wrapped) {
        (ContentKind::SmallError, _) => " j/k or ↑/↓:scroll  Esc/q:close",
        (ContentKind::Yaml, true) => {
            " drag:copy  j/k or ↑/↓:vertical  w:unwrap  /:find  n/N:matches  Esc:close"
        }
        (ContentKind::Yaml, false) => {
            " drag:copy  j/k or ↑/↓:vertical  h/l or ←/→:horizontal  w:wrap  /:find  n/N:matches  Esc:close"
        }
        (ContentKind::Describe | ContentKind::Events | ContentKind::Error, true) => {
            " drag:copy  j/k or ↑/↓:vertical  /:find  n/N:matches  Esc:close"
        }
        (ContentKind::Describe | ContentKind::Events | ContentKind::Error, false) => {
            " drag:copy  j/k or ↑/↓:vertical  h/l or ←/→:horizontal  /:find  n/N:matches  Esc:close"
        }
    }
}

pub(super) fn yaml_line<'a>(line: &'a str, query: &str, theme: &Theme) -> Line<'a> {
    if case_insensitive_regex(query).is_some_and(|expression| expression.is_match(line)) {
        return highlighted_line(line, query, Style::default(), theme);
    }
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') || trimmed == "---" {
        return Line::styled(line, Style::default().add_modifier(Modifier::DIM));
    }
    if let Some(colon) = line.find(':') {
        let (key, value) = line.split_at(colon);
        if value == ":" {
            return Line::styled(line, theme.syntax_heading());
        }
        return Line::from(vec![
            Span::styled(key, theme.syntax_key()),
            Span::styled(":", Style::default().add_modifier(Modifier::DIM)),
            Span::styled(&value[1..], yaml_value_style(value[1..].trim(), theme)),
        ]);
    }
    Line::styled(line, yaml_value_style(trimmed, theme))
}

pub(super) fn yaml_value_style(value: &str, theme: &Theme) -> Style {
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

pub(super) fn highlighted_line<'a>(
    line: &'a str,
    query: &str,
    base: Style,
    theme: &Theme,
) -> Line<'a> {
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

pub(super) fn move_modal_match(
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

pub(super) fn case_insensitive_regex(query: &str) -> Option<regex::Regex> {
    (!query.is_empty())
        .then(|| {
            RegexBuilder::new(&regex::escape(query))
                .case_insensitive(true)
                .build()
                .ok()
        })
        .flatten()
}
pub(super) fn bordered_block<'a>(title: impl Into<Line<'a>>, theme: &Theme) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border())
        .title_style(theme.title())
        .title(title)
}

pub(super) fn destructive_block<'a>(title: impl Into<Line<'a>>, theme: &Theme) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.danger())
        .title_style(theme.danger())
        .title(title)
}

pub(super) fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

pub(super) fn context_menu_area(area: Rect, column: u16, row: u16) -> Rect {
    let width = 14.min(area.width);
    let height = 7.min(area.height);
    let max_x = area.right().saturating_sub(width);
    let below = row.saturating_add(1);
    let y = if below.saturating_add(height) <= area.bottom() {
        below
    } else {
        row.saturating_sub(height).max(area.y)
    };
    Rect::new(
        column.saturating_add(1).clamp(area.x, max_x),
        y,
        width,
        height,
    )
}

pub(super) fn content_modal_area(area: Rect, kind: ContentKind) -> Rect {
    match kind {
        ContentKind::Describe | ContentKind::Yaml | ContentKind::Events => area,
        ContentKind::Error => centered(
            area,
            area.width.saturating_mul(90) / 100,
            area.height.saturating_mul(80) / 100,
        ),
        ContentKind::SmallError => centered(area, 72, 12),
    }
}

pub(super) fn content_modal_body(area: Rect, kind: ContentKind) -> Rect {
    let area = content_modal_area(area, kind);
    let inner = Block::default().borders(Borders::ALL).inner(area);
    Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner)[0]
}
