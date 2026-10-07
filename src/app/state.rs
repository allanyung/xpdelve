use super::render::{fuzzy_score, resource_kind_label};
use super::*;

fn maybe_add_command(
    entries: &mut Vec<(usize, PaletteEntry)>,
    query: &str,
    label: &str,
    action: PaletteAction,
) {
    if !query.is_empty()
        && let Some(score) = fuzzy_score(label, query)
    {
        entries.push((
            score,
            PaletteEntry {
                label: label.into(),
                action,
            },
        ));
    }
}

impl App {
    pub(super) fn new(resource: String, config: Config, cli: &Cli, theme: Theme) -> Self {
        Self {
            resource,
            snapshot: None,
            selection: Selection::empty(),
            resource_scroll: 0,
            resource_horizontal_scroll: 0,
            show_logical_name: config.ui.show_logical_name,
            collapsed: HashSet::new(),
            excluded_kinds: HashSet::new(),
            mode: InputMode::Normal,
            help_scroll: 0,
            input: String::new(),
            filter: String::new(),
            kind_filter: None,
            health_filter: HealthFilter::All,
            palette_selected: 0,
            find: String::new(),
            loading: false,
            paused: false,
            generation: 0,
            last_refresh: None,
            status: "Starting trace...".into(),
            quit: false,
            config,
            config_path: cli.config.clone().unwrap_or_else(config::default_path),
            cli: cli.clone(),
            no_watch: cli.no_watch,
            theme,
            kubernetes: None,
            kubernetes_status: "Kubernetes client initializing".into(),
            modal: None,
            active_mutations: HashSet::new(),
            refresh_pending: false,
            retry_delay: Duration::from_secs(1),
            resource_missing: false,
            namespace: cli.namespace.clone(),
            context: cli.context.clone(),
            toast: None,
            tree_selection: None,
            text_drag: None,
            deferred_snapshot: None,
            last_tree_click: None,
        }
    }

    pub(super) fn show_toast(&mut self, message: impl Into<String>) {
        self.toast = Some(Toast {
            message: message.into(),
            expires_at: Instant::now() + TOAST_DURATION,
        });
    }

    pub(super) fn visible(&self) -> Vec<usize> {
        self.snapshot.as_ref().map_or_else(Vec::new, |snapshot| {
            snapshot.visible_indices(
                &self.collapsed,
                &self.excluded_kinds,
                (!self.filter.is_empty()).then_some(self.filter.as_str()),
                self.kind_filter.as_ref(),
                self.health_filter,
            )
        })
    }

    pub(super) fn palette_entries(&self) -> Vec<PaletteEntry> {
        let mut kinds = self
            .snapshot
            .iter()
            .flat_map(|snapshot| snapshot.nodes.iter())
            .map(|node| node.resource_kind.clone())
            .collect::<Vec<_>>();
        kinds.sort_by(|left, right| {
            left.kind
                .to_lowercase()
                .cmp(&right.kind.to_lowercase())
                .then_with(|| left.group.to_lowercase().cmp(&right.group.to_lowercase()))
        });
        kinds.dedup();
        let kind_counts = kinds.iter().fold(HashMap::new(), |mut counts, kind| {
            *counts.entry(kind.kind.clone()).or_insert(0usize) += 1;
            counts
        });
        let query = self.input.trim();
        let mut entries = kinds
            .into_iter()
            .filter_map(|kind| {
                let qualified = kind_counts.get(kind.kind.as_str()).copied().unwrap_or(0) > 1;
                let label = resource_kind_label(&kind, qualified);
                fuzzy_score(&label, query).map(|score| {
                    (
                        score,
                        PaletteEntry {
                            label,
                            action: PaletteAction::FilterKind(kind),
                        },
                    )
                })
            })
            .collect::<Vec<_>>();
        maybe_add_command(&mut entries, query, "clear", PaletteAction::ClearKindFilter);
        maybe_add_command(
            &mut entries,
            query,
            "exclude",
            PaletteAction::OpenExcludePicker,
        );
        maybe_add_command(
            &mut entries,
            query,
            "health",
            PaletteAction::OpenHealthPicker,
        );
        maybe_add_command(&mut entries, query, "skin", PaletteAction::OpenSkinPicker);
        maybe_add_command(&mut entries, query, "reload", PaletteAction::ReloadConfig);
        maybe_add_command(&mut entries, query, "quit", PaletteAction::Quit);
        entries.sort_by(|(left_score, left), (right_score, right)| {
            right_score
                .cmp(left_score)
                .then_with(|| left.label.to_lowercase().cmp(&right.label.to_lowercase()))
        });
        entries.into_iter().map(|(_, entry)| entry).collect()
    }

    pub(super) fn kind_filter_label(&self, kind: &ResourceKind) -> String {
        let groups = self
            .snapshot
            .iter()
            .flat_map(|snapshot| snapshot.nodes.iter())
            .filter(|node| node.resource_kind.kind == kind.kind)
            .map(|node| node.resource_kind.group.as_str())
            .collect::<HashSet<_>>();
        resource_kind_label(kind, groups.len() > 1)
    }

    pub(super) fn open_palette(&mut self) {
        self.mode = InputMode::Command;
        self.input.clear();
        self.palette_selected = 0;
    }

    pub(super) fn exclusion_kinds(&self) -> Vec<ResourceKind> {
        let mut kinds = self.excluded_kinds.clone();
        kinds.extend(
            self.snapshot
                .iter()
                .flat_map(|snapshot| snapshot.nodes.iter())
                .map(|node| node.resource_kind.clone()),
        );
        let mut kinds = kinds.into_iter().collect::<Vec<_>>();
        kinds.sort_by(|left, right| {
            (left.kind != "Usage")
                .cmp(&(right.kind != "Usage"))
                .then_with(|| left.kind.to_lowercase().cmp(&right.kind.to_lowercase()))
                .then_with(|| left.group.to_lowercase().cmp(&right.group.to_lowercase()))
        });
        kinds
    }

    pub(super) fn open_exclude_picker(&mut self) {
        self.mode = InputMode::Normal;
        self.input.clear();
        self.modal = Some(Modal::ExcludePicker {
            kinds: self.exclusion_kinds(),
            excluded: self.excluded_kinds.clone(),
            cursor: 0,
            scroll: 0,
        });
    }

    pub(super) fn execute_palette_action(&mut self, action: PaletteAction) {
        match action {
            PaletteAction::FilterKind(kind) => self.kind_filter = Some(kind),
            PaletteAction::ClearKindFilter => self.kind_filter = None,
            PaletteAction::OpenExcludePicker => {
                self.open_exclude_picker();
                return;
            }
            PaletteAction::OpenHealthPicker => {
                self.mode = InputMode::Normal;
                self.input.clear();
                let selected = HEALTH_FILTERS
                    .iter()
                    .position(|filter| *filter == self.health_filter)
                    .unwrap_or_default();
                self.modal = Some(Modal::HealthPicker { selected });
                return;
            }
            PaletteAction::OpenSkinPicker => {
                self.mode = InputMode::Normal;
                self.input.clear();
                self.modal = Some(Modal::SkinPicker { selected: 0 });
                return;
            }
            PaletteAction::ReloadConfig => {
                self.mode = InputMode::Normal;
                self.input.clear();
                self.reload_config();
                return;
            }
            PaletteAction::Quit => self.quit = true,
        }
        self.mode = InputMode::Normal;
        self.input.clear();
        self.set_selection(0);
    }

    pub(super) fn reload_config(&mut self) {
        let mut cli = self.cli.clone();
        cli.config = Some(self.config_path.clone());
        let result = Config::load(&cli).and_then(|config| {
            let theme = Theme::resolve(&config.skin, config.ui.color)?;
            Ok((config, theme))
        });
        match result {
            Ok((config, theme)) => {
                if config.ui.show_logical_name != self.config.ui.show_logical_name {
                    self.show_logical_name = config.ui.show_logical_name;
                }
                self.config = config;
                self.theme = theme;
                self.retry_delay = self.retry_delay.min(Duration::from_secs(
                    self.config.trace.retry_backoff_max_seconds,
                ));
                self.tree_selection = None;
                self.last_tree_click = None;
                self.status = "Configuration reloaded".into();
            }
            Err(error) => {
                self.status = "Could not reload configuration".into();
                self.modal = Some(Modal::Text {
                    title: "Configuration reload failed".into(),
                    content: text::sanitize(&format!("{error:#}")).into(),
                    kind: ContentKind::SmallError,
                    state: TextModalState::new(ContentKind::SmallError),
                    selection: None,
                });
            }
        }
    }

    pub(super) fn apply_skin(&mut self, name: &str) {
        let mut skin = self.config.skin.clone();
        skin.name = Some(name.to_owned());
        let theme = match Theme::resolve(&skin, self.config.ui.color) {
            Ok(theme) => theme,
            Err(error) => {
                self.status = format!("Could not apply skin: {error}");
                return;
            }
        };
        if let Err(error) = config::persist_skin(&self.config_path, name) {
            self.status = format!("Could not save skin: {error:#}");
            return;
        }
        self.config.skin = skin;
        self.theme = theme;
        self.status = format!("Skin: {name}");
    }

    pub(super) fn selected_node(&self) -> Option<&ProjectedNode> {
        let visible = self.visible();
        let index = *visible.get(self.selection.visible_index)?;
        self.snapshot.as_ref()?.nodes.get(index)
    }

    pub(super) fn selected_target(&self) -> Option<Target> {
        self.selected_node().map(|node| Target {
            identity: node.identity.clone(),
            expected_uid: node.uid.clone(),
        })
    }

    pub(super) fn show_status(&mut self, identity: &Identity) {
        let Some(node) = self.snapshot.as_ref().and_then(|snapshot| {
            snapshot
                .by_identity
                .get(identity)
                .map(|index| &snapshot.nodes[*index])
        }) else {
            return;
        };
        let title = format!("Status: {}", node.identity);
        let content = node.object.get("status").map_or_else(
            || "No status reported.\n".to_owned(),
            |status| {
                serde_yaml::to_string(status)
                    .map(|yaml| text::sanitize(&yaml))
                    .unwrap_or_else(|error| {
                        text::sanitize(&format!("Could not render status: {error}\n"))
                    })
            },
        );
        self.modal = Some(Modal::Text {
            title,
            content: content.into(),
            kind: ContentKind::Yaml,
            state: TextModalState::new(ContentKind::Yaml),
            selection: None,
        });
    }

    pub(super) fn show_resource_details(&mut self) {
        let Some(node) = self.selected_node() else {
            return;
        };
        let Some(content) = &node.status_details else {
            return;
        };
        self.modal = Some(Modal::Text {
            title: format!("Resource details: {}", node.identity),
            content: content.clone().into(),
            kind: ContentKind::Error,
            state: TextModalState::new(ContentKind::Error),
            selection: None,
        });
    }

    pub(super) fn finish_action(
        &mut self,
        label: &str,
        identity: Option<Identity>,
        result: Result<Option<String>, String>,
    ) -> bool {
        if let Some(identity) = &identity {
            self.active_mutations.remove(identity);
        }
        match result {
            Ok(Some(content)) => {
                let kind = match label {
                    label if label.starts_with("YAML:") => ContentKind::Yaml,
                    label if label.starts_with("Events:") => ContentKind::Events,
                    _ => ContentKind::Describe,
                };
                self.modal = Some(Modal::Text {
                    title: label.to_owned(),
                    content: content.into(),
                    kind,
                    state: TextModalState::new(kind),
                    selection: None,
                });
                true
            }
            Ok(None) => {
                self.status = format!("{label} succeeded");
                true
            }
            Err(error) => {
                if let Some(identity) = identity {
                    self.status = format!("{label} failed");
                    self.modal = Some(Modal::Text {
                        title: format!("{label} failed"),
                        content: text::sanitize(&format!("Resource: {identity}\n\n{error}\n"))
                            .into(),
                        kind: ContentKind::Error,
                        state: TextModalState::new(ContentKind::Error),
                        selection: None,
                    });
                } else {
                    self.status = text::sanitize(&format!("{label} failed: {error}"));
                }
                false
            }
        }
    }

    pub(super) fn apply_snapshot(&mut self, snapshot: Snapshot) {
        if self
            .tree_selection
            .as_ref()
            .is_some_and(|selection| selection.selecting)
        {
            self.deferred_snapshot = Some(snapshot);
            self.loading = false;
            return;
        }
        self.last_tree_click = None;
        let selection_chain = self.selection_chain();
        let snapshot = Arc::new(snapshot);
        self.snapshot = Some(Arc::clone(&snapshot));
        self.collapsed
            .retain(|identity| snapshot.by_identity.contains_key(identity));
        let visible = self.visible();
        let visible_index = selection_chain
            .iter()
            .find_map(|identity| snapshot.by_identity.get(identity))
            .and_then(|index| {
                visible
                    .iter()
                    .position(|visible_index| visible_index == index)
            })
            .unwrap_or_else(|| {
                self.selection
                    .visible_index
                    .min(visible.len().saturating_sub(1))
            });
        self.selection = Selection::new(visible_index, &snapshot.nodes, &visible);
        if let Some(selection) = &self.tree_selection
            && super::render::rendered_tree(self, selection.area)
                .is_none_or(|rendered| rendered.content != selection.content)
        {
            self.tree_selection = None;
        }
        self.loading = false;
        self.resource_missing = false;
        self.last_refresh = Some(Instant::now());
        self.status = format!("Trace updated: {} resources", snapshot.nodes.len());
        self.retry_delay = Duration::from_secs(1);
    }

    pub(super) fn apply_deferred_snapshot(&mut self) {
        if !self
            .tree_selection
            .as_ref()
            .is_some_and(|selection| selection.selecting)
            && let Some(snapshot) = self.deferred_snapshot.take()
        {
            self.apply_snapshot(snapshot);
        }
    }

    pub(super) fn apply_resource_not_found(&mut self) {
        self.deferred_snapshot = None;
        self.snapshot = None;
        self.selection = Selection::empty();
        self.resource_scroll = 0;
        self.resource_horizontal_scroll = 0;
        self.collapsed.clear();
        if !matches!(
            self.modal,
            Some(Modal::Text {
                kind: ContentKind::Error | ContentKind::SmallError,
                ..
            })
        ) {
            self.modal = None;
        }
        self.refresh_pending = false;
        self.loading = false;
        self.resource_missing = true;
        self.status.clear();
        self.tree_selection = None;
        self.last_tree_click = None;
        self.retry_delay = Duration::from_secs(1);
    }

    pub(super) fn selection_chain(&self) -> Vec<Identity> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        let visible = self.visible();
        let Some(mut index) = visible.get(self.selection.visible_index).copied() else {
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

    pub(super) fn move_selection(&mut self, delta: isize) {
        let Some(snapshot) = &self.snapshot else {
            self.selection = Selection::empty();
            return;
        };
        let visible = self.visible();
        if visible.is_empty() {
            self.selection = Selection::empty();
            return;
        }
        let visible_index = self
            .selection
            .visible_index
            .saturating_add_signed(delta)
            .min(visible.len().saturating_sub(1));
        self.selection = Selection::new(visible_index, &snapshot.nodes, &visible);
    }

    pub(super) fn set_selection(&mut self, index: usize) {
        let Some(snapshot) = &self.snapshot else {
            self.selection = Selection::empty();
            return;
        };
        let visible = self.visible();
        let visible_index = index.min(visible.len().saturating_sub(1));
        self.selection = Selection::new(visible_index, &snapshot.nodes, &visible);
    }

    pub(super) fn ensure_selection_visible(&mut self, viewport: usize) {
        self.resource_scroll = self.resource_view_start(viewport);
    }

    pub(super) fn resource_view_start(&self, viewport: usize) -> usize {
        let len = self.visible().len();
        let max = len.saturating_sub(viewport);
        Scroller::with_offset(self.resource_scroll, max)
            .ensure_visible(self.selection.visible_index, viewport)
            .offset()
    }

    pub(super) fn expand_or_child(&mut self) {
        let visible = self.visible();
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let Some(index) = visible.get(self.selection.visible_index).copied() else {
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

    pub(super) fn collapse_or_parent(&mut self) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let visible = self.visible();
        let Some(index) = visible.get(self.selection.visible_index).copied() else {
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

    pub(super) fn find_next(&mut self, reverse: bool) {
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
                node.matches_text(&needle).then_some(position)
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
                .find(|position| *position < self.selection.visible_index)
                .unwrap_or_else(|| *matches.last().expect("matches is not empty"))
        } else {
            matches
                .iter()
                .copied()
                .find(|position| *position > self.selection.visible_index)
                .unwrap_or(matches[0])
        };
        self.set_selection(next);
    }

    pub(super) fn submit_input(&mut self) {
        match self.mode {
            InputMode::Filter => self.filter = self.input.trim().to_owned(),
            InputMode::Find => {
                self.find = self.input.trim().to_owned();
                self.find_next(false);
            }
            InputMode::Normal | InputMode::Command | InputMode::Help => {}
        }
        self.mode = InputMode::Normal;
        self.input.clear();
        self.set_selection(self.selection.visible_index);
    }
}
