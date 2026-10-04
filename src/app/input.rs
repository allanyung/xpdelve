use super::render::*;
use super::selection::*;
use super::*;

fn vertical_wheel_delta(mouse: MouseEvent, count: usize) -> Option<isize> {
    if mouse.modifiers.contains(KeyModifiers::SHIFT) {
        return None;
    }
    let distance = count.saturating_mul(3).try_into().unwrap_or(isize::MAX);
    match mouse.kind {
        MouseEventKind::ScrollUp => Some(-distance),
        MouseEventKind::ScrollDown => Some(distance),
        _ => None,
    }
}

fn scroll_vertical(scroll: u16, delta: isize, max_scroll: u16) -> u16 {
    Scroller::with_offset(usize::from(scroll), usize::from(max_scroll))
        .scroll_by(delta)
        .offset_u16()
}

impl App {
    pub(super) fn clamp_tree_horizontal_scroll(&mut self, terminal_area: Rect) {
        let Some(tree_area) = resource_tree_area(terminal_area) else {
            self.tree_selection = None;
            self.last_tree_click = None;
            return;
        };
        let offset = if self.resource_horizontal_scroll == 0 {
            0
        } else {
            tree_table_plan(self, tree_area, &self.visible()).map_or(0, |plan| {
                plan.horizontal_layout(self.resource_horizontal_scroll)
                    .offset
            })
        };
        if offset != self.resource_horizontal_scroll
            || self
                .tree_selection
                .as_ref()
                .is_some_and(|selection| selection.area != tree_area)
        {
            self.tree_selection = None;
            self.last_tree_click = None;
        }
        self.resource_horizontal_scroll = offset;
    }

    fn scroll_tree_horizontal(&mut self, terminal_area: Rect, right: bool, count: usize) {
        if resource_tree_area(terminal_area).is_none() {
            return;
        }
        self.clamp_tree_horizontal_scroll(terminal_area);
        self.resource_horizontal_scroll = if right {
            self.resource_horizontal_scroll
                .saturating_add(count.saturating_mul(4))
        } else {
            self.resource_horizontal_scroll
                .saturating_sub(count.saturating_mul(4))
        };
        self.clamp_tree_horizontal_scroll(terminal_area);
        self.tree_selection = None;
        self.last_tree_click = None;
    }

    pub(super) fn handle_mouse(
        &mut self,
        mouse: MouseEvent,
        terminal_area: Rect,
    ) -> Option<MouseAction> {
        self.handle_mouse_events(mouse, 1, terminal_area)
    }

    pub(super) fn handle_mouse_events(
        &mut self,
        mouse: MouseEvent,
        count: usize,
        terminal_area: Rect,
    ) -> Option<MouseAction> {
        if self.modal.is_none() && self.mode == InputMode::Help {
            let area = centered(terminal_area, 72, 20);
            if area.contains((mouse.column, mouse.row).into())
                && let Some(delta) = vertical_wheel_delta(mouse, count)
            {
                let max_scroll =
                    (HELP_LINES.len() as u16).saturating_sub(area.height.saturating_sub(3));
                self.help_scroll = scroll_vertical(self.help_scroll, delta, max_scroll);
            }
            return None;
        }
        if self.modal.is_none() {
            let action = self.handle_tree_mouse(mouse, count, terminal_area);
            self.apply_deferred_snapshot();
            return action;
        }
        if matches!(self.modal, Some(Modal::ContextMenu { .. })) {
            return self.handle_context_menu_mouse(mouse, terminal_area);
        }
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
        if let Some(delta) = vertical_wheel_delta(mouse, count) {
            if content_modal_area(terminal_area, *kind).contains((mouse.column, mouse.row).into()) {
                let (max_scroll, _) = content.scroll_bounds(body.width, body.height, *wrapped);
                *vertical_scroll = scroll_vertical(*vertical_scroll, delta, max_scroll);
                *selection = None;
            }
            return None;
        }
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
                    dragged: false,
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
                    active.dragged = true;
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let mut active = selection.take()?;
                if !active.dragged {
                    return None;
                }
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
                *selection = Some(active);
                if !range.is_empty() {
                    return Some(MouseAction::Copy(content[range].to_owned()));
                }
            }
            _ => {}
        }
        None
    }

    pub(super) fn handle_context_menu_mouse(
        &mut self,
        mouse: MouseEvent,
        terminal_area: Rect,
    ) -> Option<MouseAction> {
        let Some(Modal::ContextMenu {
            target,
            column,
            row,
            pressed,
        }) = &self.modal
        else {
            return None;
        };
        let target = target.clone();
        let menu = context_menu_area(terminal_area, *column, *row);
        let inner = Block::default().borders(Borders::ALL).inner(menu);
        let selected = (mouse.column >= inner.x
            && mouse.column < inner.x.saturating_add(inner.width)
            && mouse.row >= inner.y
            && mouse.row < inner.y.saturating_add(inner.height))
        .then(|| usize::from(mouse.row - inner.y));
        let pressed = *pressed;
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(Modal::ContextMenu { pressed, .. }) = &mut self.modal {
                    *pressed = selected;
                }
                if selected.is_none() {
                    self.modal = None;
                }
                return None;
            }
            MouseEventKind::Up(MouseButton::Left) if selected == pressed => {}
            MouseEventKind::Up(MouseButton::Left) => {
                self.modal = None;
                return None;
            }
            _ => return None,
        }
        self.modal = None;
        match selected {
            Some(0) => Some(MouseAction::Action(UiAction::Yaml(target))),
            Some(1) if self.config.read_only => {
                self.status = "Edit is disabled in read-only mode".into();
                None
            }
            Some(1) => Some(MouseAction::Action(UiAction::Edit(target))),
            Some(2) => {
                self.show_status(&target.identity);
                None
            }
            Some(3) => Some(MouseAction::Action(UiAction::Events(target))),
            Some(4) => Some(MouseAction::Action(UiAction::Describe(target))),
            _ => None,
        }
    }

    pub(super) fn handle_tree_mouse(
        &mut self,
        mouse: MouseEvent,
        count: usize,
        terminal_area: Rect,
    ) -> Option<MouseAction> {
        if !self.captures_mouse() {
            return None;
        }
        let tree_area = resource_tree_area(terminal_area)?;
        if mouse.column >= tree_area.x
            && mouse.column < tree_area.right()
            && mouse.row >= tree_area.y
            && mouse.row < tree_area.bottom()
        {
            let right = match mouse.kind {
                MouseEventKind::ScrollLeft => Some(false),
                MouseEventKind::ScrollRight => Some(true),
                MouseEventKind::ScrollUp if mouse.modifiers.contains(KeyModifiers::SHIFT) => {
                    Some(false)
                }
                MouseEventKind::ScrollDown if mouse.modifiers.contains(KeyModifiers::SHIFT) => {
                    Some(true)
                }
                _ => None,
            };
            if let Some(right) = right {
                self.scroll_tree_horizontal(terminal_area, right, count);
                return None;
            }
            if let Some(delta) = vertical_wheel_delta(mouse, count) {
                self.tree_selection = None;
                self.last_tree_click = None;
                self.move_selection(delta);
                self.ensure_selection_visible(usize::from(tree_area.height.saturating_sub(3)));
                return None;
            }
        }
        let body = Block::default().borders(Borders::ALL).inner(tree_area);
        let rendered = rendered_tree(self, tree_area)?;

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let content_rows = u16::try_from(rendered.row_count)
                    .unwrap_or(u16::MAX)
                    .saturating_add(1);
                if mouse.row < body.y
                    || mouse.row >= body.y.saturating_add(content_rows.min(body.height))
                {
                    self.tree_selection = None;
                    return None;
                }
                self.tree_selection = selection_point_at(
                    &rendered.content,
                    body,
                    mouse.column,
                    mouse.row,
                    0,
                    0,
                    false,
                    false,
                )
                .map(|point| TreeSelection {
                    text: TextSelection {
                        anchor: point,
                        focus: point,
                        dragged: false,
                    },
                    content: rendered.content,
                    area: tree_area,
                    selecting: true,
                });
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                self.last_tree_click = None;
                if let Some(active) = &mut self.tree_selection
                    && let Some(point) = selection_point_at(
                        &active.content,
                        body,
                        mouse.column,
                        mouse.row,
                        0,
                        0,
                        false,
                        true,
                    )
                {
                    active.text.focus = point;
                    active.text.dragged = true;
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let mut active = self.tree_selection.take()?;
                active.selecting = false;
                if !active.text.dragged {
                    if let Some(position) = clicked_tree_position(
                        body,
                        mouse.column,
                        mouse.row,
                        rendered.start,
                        rendered.row_count,
                    ) {
                        self.set_selection(position);
                        let identity = self.selection.identity.clone()?;
                        let now = Instant::now();
                        let double_click = self.last_tree_click.as_ref().is_some_and(|click| {
                            click.identity == identity
                                && now.duration_since(click.at) <= DOUBLE_CLICK_INTERVAL
                        });
                        if double_click {
                            self.last_tree_click = None;
                            return self
                                .selected_target()
                                .map(UiAction::Yaml)
                                .map(MouseAction::Action);
                        }
                        self.last_tree_click = Some(TreeClick { identity, at: now });
                    }
                    return None;
                }
                if let Some(point) = selection_point_at(
                    &active.content,
                    body,
                    mouse.column,
                    mouse.row,
                    0,
                    0,
                    false,
                    true,
                ) {
                    active.text.focus = point;
                }
                let range = active.text.range();
                let value = (!range.is_empty())
                    .then(|| MouseAction::Copy(active.content[range].to_owned()));
                self.tree_selection = Some(active);
                return value;
            }
            MouseEventKind::Down(MouseButton::Right) => {
                if let Some(position) = clicked_tree_position(
                    body,
                    mouse.column,
                    mouse.row,
                    rendered.start,
                    rendered.row_count,
                ) {
                    self.set_selection(position);
                    self.tree_selection = None;
                    self.last_tree_click = None;
                    if let Some(target) = self.selected_target() {
                        self.modal = Some(Modal::ContextMenu {
                            target,
                            column: mouse.column,
                            row: mouse.row,
                            pressed: None,
                        });
                    }
                }
            }
            _ => {}
        }
        None
    }

    pub(super) fn captures_mouse(&self) -> bool {
        match &self.modal {
            Some(Modal::Text { kind, .. }) => kind.supports_mouse_selection(),
            Some(Modal::ContextMenu { .. }) => true,
            Some(
                Modal::Delete { .. }
                | Modal::Finalizers { .. }
                | Modal::SkinPicker { .. }
                | Modal::HealthPicker { .. }
                | Modal::ExcludePicker { .. },
            ) => false,
            None => {
                self.mode == InputMode::Help
                    || (self.mode != InputMode::Command
                        && self.snapshot.is_some()
                        && !self.resource_missing)
            }
        }
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent, terminal_area: Rect) -> UiAction {
        let page_size = terminal_area.height.saturating_sub(6) as usize;
        if key.kind != KeyEventKind::Press {
            return UiAction::None;
        }
        if self.modal.is_none() {
            self.tree_selection = None;
            self.last_tree_click = None;
        }
        if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
            self.quit = true;
            return UiAction::None;
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
                    if search_input.is_none()
                        && matches!(key.code, KeyCode::Esc | KeyCode::Char('q'))
                    {
                        self.modal = None;
                        return UiAction::None;
                    }
                    let modal_area = content_modal_area(terminal_area, *kind);
                    let body_width = modal_area.width.saturating_sub(2) as usize;
                    let body_height = modal_area.height.saturating_sub(3) as usize;
                    let (max_vertical, max_horizontal) = content.scroll_bounds(
                        modal_area.width.saturating_sub(2),
                        modal_area.height.saturating_sub(3),
                        *wrapped,
                    );
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
                Modal::ContextMenu { .. } => {
                    if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
                        self.modal = None;
                    }
                }
                Modal::SkinPicker { selected } => {
                    let mut picker = ListPicker::with_cursor(
                        *selected,
                        crate::theme::BUILTIN_NAMES.len(),
                    );
                    match picker.handle_key(key, 10) {
                        ListPickerAction::Close => self.modal = None,
                        ListPickerAction::Select(index) => {
                            let name = crate::theme::BUILTIN_NAMES[index].to_owned();
                            self.modal = None;
                            self.apply_skin(&name);
                        }
                        ListPickerAction::None => *selected = picker.cursor(),
                    }
                }
                Modal::HealthPicker { selected } => {
                    let mut picker = ListPicker::with_cursor(*selected, HEALTH_FILTERS.len());
                    match picker.handle_key(key, 3) {
                        ListPickerAction::Close => self.modal = None,
                        ListPickerAction::Select(index) => {
                            self.health_filter = HEALTH_FILTERS[index];
                            self.status = format!(
                                "Health filter: {}",
                                health_filter_label(self.health_filter)
                            );
                            self.modal = None;
                            self.set_selection(0);
                        }
                        ListPickerAction::None => *selected = picker.cursor(),
                    }
                }
                Modal::ExcludePicker {
                    kinds,
                    excluded,
                    cursor,
                    scroll,
                } => {
                    let viewport = usize::from(
                        exclude_picker_area(terminal_area, kinds.len())
                            .height
                            .saturating_sub(3),
                    )
                    .max(1);
                    let mut picker = ListPicker::with_cursor(*cursor, kinds.len());
                    match picker.handle_key(key, viewport) {
                        ListPickerAction::Close => {
                            self.modal = None;
                            return UiAction::None;
                        }
                        ListPickerAction::Select(_) => {
                            self.excluded_kinds.clone_from(excluded);
                            let count = self.excluded_kinds.len();
                            self.status = match count {
                                0 => "All resource kinds shown".into(),
                                1 => "1 resource kind excluded".into(),
                                _ => format!("{count} resource kinds excluded"),
                            };
                            self.modal = None;
                            self.set_selection(self.selection.visible_index);
                            return UiAction::None;
                        }
                        ListPickerAction::None => {
                            match key.code {
                                KeyCode::Char(' ') => {
                                    if let Some(kind) = kinds.get(picker.cursor())
                                        && !excluded.remove(kind)
                                    {
                                        excluded.insert(kind.clone());
                                    }
                                }
                                KeyCode::Char('a') => excluded.clear(),
                                KeyCode::Char('x') => {
                                    excluded.extend(kinds.iter().cloned());
                                }
                                KeyCode::Char('o') => {
                                    if let Some(visible) = kinds.get(picker.cursor()).cloned() {
                                        excluded.extend(kinds.iter().cloned());
                                        excluded.remove(&visible);
                                    }
                                }
                                _ => {}
                            }
                            *cursor = picker.cursor();
                            if !kinds.is_empty() {
                                if *cursor < *scroll {
                                    *scroll = *cursor;
                                } else if *cursor >= scroll.saturating_add(viewport) {
                                    *scroll = cursor.saturating_add(1).saturating_sub(viewport);
                                }
                            }
                        }
                    }
                }
            }
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
        if self.mode == InputMode::Command {
            let entries = self.palette_entries();
            match key.code {
                KeyCode::Esc => {
                    self.mode = InputMode::Normal;
                    self.input.clear();
                }
                KeyCode::Enter => {
                    let selected = self.palette_selected.min(entries.len().saturating_sub(1));
                    if let Some(entry) = entries.get(selected) {
                        self.execute_palette_action(entry.action.clone());
                    }
                }
                KeyCode::Down => {
                    if !entries.is_empty() {
                        self.palette_selected = (self.palette_selected + 1) % entries.len();
                    }
                }
                KeyCode::Up => {
                    if !entries.is_empty() {
                        self.palette_selected = self
                            .palette_selected
                            .checked_sub(1)
                            .unwrap_or(entries.len() - 1);
                    }
                }
                KeyCode::Backspace => {
                    self.input.pop();
                    self.palette_selected = 0;
                }
                KeyCode::Char(character) => {
                    self.input.push(character);
                    self.palette_selected = 0;
                }
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
            (KeyCode::Enter, _) => self.show_resource_details(),
            (KeyCode::Right, KeyModifiers::SHIFT) | (KeyCode::Char('l'), KeyModifiers::NONE) => {
                self.scroll_tree_horizontal(terminal_area, true, 1);
            }
            (KeyCode::Left, KeyModifiers::SHIFT) | (KeyCode::Char('h'), KeyModifiers::NONE) => {
                self.scroll_tree_horizontal(terminal_area, false, 1);
            }
            (KeyCode::Right, _) => self.expand_or_child(),
            (KeyCode::Left, _) => self.collapse_or_parent(),
            (KeyCode::Char(']'), _) => self.collapsed.clear(),
            (KeyCode::Char('['), _) => {
                if let Some(snapshot) = &self.snapshot {
                    self.collapsed = snapshot
                        .nodes
                        .iter()
                        .filter(|node| node.parent.is_some() && node.child_count > 0)
                        .map(|node| node.identity.clone())
                        .collect();
                }
                self.set_selection(0);
            }
            (KeyCode::Char('/'), _) => {
                self.mode = InputMode::Filter;
                self.input.clone_from(&self.filter);
            }
            (KeyCode::Char(':'), _) => self.open_palette(),
            (KeyCode::Char('f'), _) => {
                self.mode = InputMode::Find;
                self.input.clone_from(&self.find);
            }
            (KeyCode::Char('n'), _) => self.find_next(false),
            (KeyCode::Char('N'), _) => self.find_next(true),
            (KeyCode::Esc, _) => {
                self.filter.clear();
                self.find.clear();
                self.kind_filter = None;
                self.health_filter = HealthFilter::All;
                self.set_selection(self.selection.visible_index);
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
            (KeyCode::Char('s'), _) => {
                if let Some(identity) = self.selection.identity.clone() {
                    self.show_status(&identity);
                }
            }
            (KeyCode::Char('E'), _) => {
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
