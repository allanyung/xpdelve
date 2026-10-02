use super::render::*;
use super::runtime::kubectl_args;
use super::selection::*;
use super::*;
use crate::model::Snapshot;
use clap::Parser;
use ratatui::backend::TestBackend;
use ratatui::style::Color;

fn app() -> App {
    app_with_theme("catppuccin-mocha", crate::config::ColorMode::Always)
}

fn resource_kind(group: &str, kind: &str) -> ResourceKind {
    ResourceKind {
        group: group.into(),
        kind: kind.into(),
    }
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

fn copied_mouse_text(action: Option<MouseAction>) -> Option<String> {
    match action {
        Some(MouseAction::Copy(value)) => Some(value),
        Some(MouseAction::Action(_)) => panic!("expected no mouse action"),
        None => None,
    }
}

#[test]
fn mutation_failure_opens_persistent_error_modal() {
    let mut app = app();
    let identity = app.selected_node().unwrap().identity.clone();
    app.active_mutations.insert(identity.clone());

    let succeeded = app.finish_action(
        "Delete (Foreground)",
        Some(identity.clone()),
        Err("API request failed: admission webhook denied the request".into()),
    );

    assert!(!succeeded);
    assert!(!app.active_mutations.contains(&identity));
    assert_eq!(app.status, "Delete (Foreground) failed");
    let Some(Modal::Text {
        title,
        content,
        kind,
        wrapped,
        ..
    }) = &app.modal
    else {
        panic!("expected persistent mutation error modal");
    };
    assert_eq!(title, "Delete (Foreground) failed");
    assert!(content.contains(&format!("Resource: {identity}")));
    assert!(content.contains("admission webhook denied the request"));
    assert_eq!(*kind, ContentKind::Error);
    assert!(*wrapped);

    app.apply_snapshot(
        Snapshot::parse(
            br#"{"object":{"apiVersion":"v1","kind":"Root","metadata":{"name":"root"}}}"#,
        )
        .unwrap(),
    );
    assert!(matches!(
        app.modal,
        Some(Modal::Text {
            kind: ContentKind::Error,
            ..
        })
    ));

    app.apply_resource_not_found();
    assert!(matches!(
        app.modal,
        Some(Modal::Text {
            kind: ContentKind::Error,
            ..
        })
    ));
    app.handle_key(
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        Rect::new(0, 0, 80, 20),
    );
    assert!(app.modal.is_none());
}

#[test]
fn inspection_failure_remains_in_status_line() {
    let mut app = app();

    let succeeded = app.finish_action(
        "Events: Root/root",
        None,
        Err("request was forbidden".into()),
    );

    assert!(!succeeded);
    assert!(app.modal.is_none());
    assert_eq!(
        app.status,
        "Events: Root/root failed: request was forbidden"
    );
}

#[test]
fn collapse_hides_descendants() {
    let mut app = app();
    assert_eq!(app.visible().len(), 2);
    app.toggle_selected();
    assert_eq!(app.visible().len(), 1);
}

#[test]
fn collapse_all_keeps_root_expanded() {
    let mut app = app();
    app.apply_snapshot(
            Snapshot::parse(
                br#"{"object":{"apiVersion":"v1","kind":"Root","metadata":{"name":"root"}},"children":[{"object":{"apiVersion":"v1","kind":"Child","metadata":{"name":"child"}},"children":[{"object":{"apiVersion":"v1","kind":"Grandchild","metadata":{"name":"grandchild"}}}]}]}"#,
            )
            .unwrap(),
        );
    let root = app.snapshot.as_ref().unwrap().nodes[0].identity.clone();
    let child = app.snapshot.as_ref().unwrap().nodes[1].identity.clone();
    app.collapsed.insert(root.clone());

    app.handle_key(
        KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE),
        Rect::new(0, 0, 100, 20),
    );

    assert!(!app.collapsed.contains(&root));
    assert!(app.collapsed.contains(&child));
    assert_eq!(app.visible(), vec![0, 1]);
    assert_eq!(app.selected_visible, 0);
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
    let mut app = app();
    let snapshot = app.snapshot.as_ref().unwrap();
    let child = &snapshot.nodes[1];
    let cell = object_cell_with_state(snapshot, child, false);
    assert!(cell.starts_with("└─── Child/child"));

    let root = &snapshot.nodes[0];
    let cell = object_cell_with_state(snapshot, root, false);
    assert!(cell.starts_with("▾ Root/root"));

    app.toggle_selected();
    let rendered = rendered_tree(&app, Rect::new(0, 0, 100, 16)).unwrap();
    assert!(rendered.lines[1].starts_with("▸ Root/root"));
    assert_eq!(
        cell.split_once("Root/root").unwrap().0.width(),
        rendered.lines[1].split_once("Root/root").unwrap().0.width(),
        "collapse state must not move the resource name"
    );
}

#[test]
fn ascii_tree_uses_fixed_width_disclosure_indicators() {
    let mut app = app();
    app.config.ui.ascii = true;

    let expanded = rendered_tree(&app, Rect::new(0, 0, 100, 16)).unwrap();
    assert!(expanded.lines[1].starts_with("- Root/root"));
    assert!(expanded.lines[2].starts_with("`--- Child/child"));

    app.toggle_selected();
    let collapsed = rendered_tree(&app, Rect::new(0, 0, 100, 16)).unwrap();
    assert!(collapsed.lines[1].starts_with("+ Root/root"));
    assert_eq!(
        expanded.lines[1].find("Root/root"),
        collapsed.lines[1].find("Root/root")
    );
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
fn command_palette_lists_unique_snapshot_kinds() {
    let mut app = app();
    let area = Rect::new(0, 0, 100, 20);

    app.handle_key(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE), area);

    assert_eq!(app.mode, InputMode::Command);
    assert_eq!(
        app.palette_entries()
            .into_iter()
            .map(|entry| entry.label)
            .collect::<Vec<_>>(),
        ["Child", "Root"]
    );
}

#[test]
fn command_palette_qualifies_duplicate_kinds_by_group() {
    let mut app = app();
    app.apply_snapshot(
            Snapshot::parse(
                br#"{
                    "object":{"apiVersion":"example.io/v1","kind":"Root","metadata":{"name":"root"}},
                    "children":[
                        {"object":{"apiVersion":"alpha.example.io/v1","kind":"Widget","metadata":{"name":"alpha"}}},
                        {"object":{"apiVersion":"beta.example.io/v1","kind":"Widget","metadata":{"name":"beta"}}}
                    ]
                }"#,
            )
            .unwrap(),
        );

    let entries = app.palette_entries();
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.label.as_str())
            .collect::<Vec<_>>(),
        ["Root", "Widget.alpha.example.io", "Widget.beta.example.io"]
    );

    app.execute_palette_action(entries[2].action.clone());
    assert_eq!(
        app.kind_filter,
        Some(resource_kind("beta.example.io", "Widget"))
    );
    assert_eq!(app.visible(), vec![2]);
    assert_eq!(app.selected_node().unwrap().identity.name, "beta");
}

#[test]
fn typing_narrows_the_command_palette_and_enter_filters_by_kind() {
    let mut app = app();
    let area = Rect::new(0, 0, 100, 20);
    for key in [':', 'c', 'h'] {
        app.handle_key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE), area);
    }

    assert_eq!(app.palette_entries().len(), 1);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), area);

    assert_eq!(app.mode, InputMode::Normal);
    assert_eq!(app.kind_filter, Some(resource_kind("", "Child")));
    assert_eq!(app.visible(), vec![1]);
    assert_eq!(app.selected_node().unwrap().identity.kind, "Child");
}

#[test]
fn kind_filtered_rows_are_rendered_without_tree_prefixes() {
    let mut app = app();
    app.kind_filter = Some(resource_kind("", "Child"));

    let rendered = rendered_tree(&app, Rect::new(0, 0, 100, 16)).unwrap();

    assert!(rendered.lines[1].starts_with("  Child/child"));
    assert!(!rendered.lines[1].contains("└─"));
}

#[test]
fn command_palette_navigation_executes_the_highlighted_action() {
    let mut app = app();
    let area = Rect::new(0, 0, 100, 20);
    app.handle_key(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE), area);
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), area);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), area);

    assert_eq!(app.kind_filter, Some(resource_kind("", "Root")));
    assert_eq!(app.visible(), vec![0]);
}

#[test]
fn clear_palette_command_removes_the_kind_filter() {
    let mut app = app();
    let area = Rect::new(0, 0, 100, 20);
    app.kind_filter = Some(resource_kind("", "Child"));
    for key in [':', 'c', 'l', 'e', 'a', 'r'] {
        app.handle_key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE), area);
    }

    let entries = app.palette_entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].action, PaletteAction::ClearKindFilter);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), area);

    assert!(app.kind_filter.is_none());
    assert_eq!(app.visible(), vec![0, 1]);
}

#[test]
fn skin_palette_command_opens_the_skin_picker() {
    let mut app = app();
    let area = Rect::new(0, 0, 100, 24);
    for key in [':', 's', 'k', 'i', 'n'] {
        app.handle_key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE), area);
    }

    let entries = app.palette_entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].action, PaletteAction::OpenSkinPicker);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), area);

    assert!(matches!(app.modal, Some(Modal::SkinPicker { selected: 0 })));
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("Skins"));
    assert!(rendered.contains("catppuccin-mocha"));
    assert!(rendered.contains("nord"));
}

#[test]
fn health_palette_command_filters_unhealthy_resources_with_ancestors() {
    let mut app = app();
    let trace = serde_json::json!({
        "object": {
            "apiVersion": "v1", "kind": "Root", "metadata": {"name": "root"},
            "status": {"conditions": [{"type": "Ready", "status": "True"}]}
        },
        "children": [
            {"object": {
                "apiVersion": "v1", "kind": "Broken", "metadata": {"name": "broken"},
                "status": {"conditions": [{"type": "Ready", "status": "False"}]}
            }},
            {"object": {
                "apiVersion": "v1", "kind": "Healthy", "metadata": {"name": "healthy"},
                "status": {"conditions": [{"type": "Ready", "status": "True"}]}
            }}
        ]
    });
    app.apply_snapshot(Snapshot::parse(trace.to_string().as_bytes()).unwrap());
    let area = Rect::new(0, 0, 100, 24);
    for key in [':', 'h', 'e', 'a', 'l', 't', 'h'] {
        app.handle_key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE), area);
    }

    let entries = app.palette_entries();
    assert_eq!(entries[0].action, PaletteAction::OpenHealthPicker);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), area);
    assert!(matches!(
        app.modal,
        Some(Modal::HealthPicker { selected: 0 })
    ));

    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), area);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), area);

    assert_eq!(app.health_filter, HealthFilter::Unhealthy);
    assert_eq!(app.visible(), vec![0, 1]);
    assert_eq!(app.selected_visible, 0);
    assert!(app.modal.is_none());

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), area);
    assert_eq!(app.health_filter, HealthFilter::All);
    assert_eq!(app.visible(), vec![0, 1, 2]);
}

#[test]
fn exclude_palette_puts_usage_first_and_applies_hidden_kinds() {
    let mut app = app();
    app.apply_snapshot(
            Snapshot::parse(
                br#"{
                    "object":{"apiVersion":"example.io/v1","kind":"Root","metadata":{"name":"root"}},
                    "children":[
                        {"object":{"apiVersion":"apps/v1","kind":"Deployment","metadata":{"name":"app"}}},
                        {"object":{"apiVersion":"protection.crossplane.io/v1beta1","kind":"Usage","metadata":{"name":"usage"}}}
                    ]
                }"#,
            )
            .unwrap(),
        );
    let area = Rect::new(0, 0, 100, 24);
    for key in [':', 'e', 'x', 'c', 'l', 'u', 'd', 'e'] {
        app.handle_key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE), area);
    }

    let entries = app.palette_entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].action, PaletteAction::OpenExcludePicker);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), area);

    let Some(Modal::ExcludePicker { kinds, .. }) = &app.modal else {
        panic!("expected exclusion picker");
    };
    assert_eq!(kinds[0].to_string(), "Usage.protection.crossplane.io");

    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    assert!(
        terminal
            .backend()
            .to_string()
            .contains("[ ] Usage.protection.crossplane.io")
    );

    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE), area);
    terminal.draw(|frame| render(frame, &app)).unwrap();
    assert!(
        terminal
            .backend()
            .to_string()
            .contains("[x] Usage.protection.crossplane.io")
    );
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), area);

    assert!(app.modal.is_none());
    assert!(app.excluded_kinds.contains(&ResourceKind {
        group: "protection.crossplane.io".into(),
        kind: "Usage".into(),
    }));
    assert_eq!(
        app.visible()
            .into_iter()
            .map(|index| app.snapshot.as_ref().unwrap().nodes[index]
                .identity
                .kind
                .as_str())
            .collect::<Vec<_>>(),
        ["Root", "Deployment"]
    );

    app.apply_snapshot(
            Snapshot::parse(
                br#"{"object":{"apiVersion":"example.io/v1","kind":"Root","metadata":{"name":"root"}},"children":[{"object":{"apiVersion":"protection.crossplane.io/v1beta1","kind":"Usage","metadata":{"name":"new-usage"}}},{"object":{"apiVersion":"v1","kind":"Service","metadata":{"name":"new-kind"}}}]}"#,
            )
            .unwrap(),
        );
    assert_eq!(app.visible(), vec![0, 2]);
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), area);
    assert_eq!(app.excluded_kinds.len(), 1);

    app.apply_snapshot(
            Snapshot::parse(
                br#"{"object":{"apiVersion":"example.io/v1","kind":"Root","metadata":{"name":"root"}}}"#,
            )
            .unwrap(),
        );
    assert_eq!(
        app.exclusion_kinds()[0].to_string(),
        "Usage.protection.crossplane.io"
    );
}

#[test]
fn exclude_picker_can_cancel_show_all_hide_all_and_show_only() {
    let mut app = app();
    let area = Rect::new(0, 0, 100, 24);

    app.open_exclude_picker();
    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE), area);
    app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE), area);
    assert!(app.excluded_kinds.is_empty());

    app.open_exclude_picker();
    app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE), area);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), area);
    assert!(app.visible().is_empty());

    app.open_exclude_picker();
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE), area);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), area);
    assert_eq!(app.visible(), vec![0, 1]);

    app.open_exclude_picker();
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), area);
    app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE), area);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), area);
    assert_eq!(app.visible(), vec![0]);
}

#[test]
fn quit_palette_command_exits_the_application() {
    let mut app = app();
    let area = Rect::new(0, 0, 100, 20);
    for key in [':', 'q', 'u', 'i', 't'] {
        app.handle_key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE), area);
    }

    let entries = app.palette_entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].action, PaletteAction::Quit);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), area);

    assert!(app.quit);
    assert_eq!(app.mode, InputMode::Normal);
}

#[test]
fn selecting_a_skin_applies_and_persists_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("xpdelve/config.toml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "# preserved\nschema_version = 1\n\n[skin]\nname = 'catppuccin-mocha'\n",
    )
    .unwrap();
    let mut app = app();
    app.config_path = path.clone();
    app.modal = Some(Modal::SkinPicker { selected: 0 });
    let area = Rect::new(0, 0, 100, 24);

    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), area);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), area);

    assert!(app.modal.is_none());
    assert_eq!(app.theme.resolved_name, "catppuccin-latte");
    assert_eq!(app.config.skin.name.as_deref(), Some("catppuccin-latte"));
    assert_eq!(app.status, "Skin: catppuccin-latte");
    let saved = std::fs::read_to_string(path).unwrap();
    assert!(saved.contains("# preserved"));
    assert!(saved.contains("name = \"catppuccin-latte\""));
}

#[test]
fn palette_command_uses_sofka_command_styling() {
    let mut app = app();
    app.open_palette();
    app.input = "c".into();
    let entries = app.palette_entries();
    let clear_index = entries
        .iter()
        .position(|entry| entry.action == PaletteAction::ClearKindFilter)
        .unwrap();
    assert!(entries.len() > 1);
    app.palette_selected = (clear_index + 1) % entries.len();
    let backend = TestBackend::new(80, 16);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|frame| render(frame, &app)).unwrap();

    let popup = palette_area(Rect::new(0, 0, 80, 16), entries.len());
    let row = popup.y + 1 + u16::try_from(clear_index).unwrap();
    let command_column = popup.x + 3;
    let tag_column = command_column + 8;
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer.cell((command_column, row)).unwrap().symbol(), ":");
    assert_eq!(
        buffer.cell((command_column, row)).unwrap().fg,
        app.theme.palette.peach
    );
    assert_eq!(buffer.cell((tag_column, row)).unwrap().symbol(), "c");
    assert_eq!(
        buffer.cell((tag_column, row)).unwrap().fg,
        app.theme.palette.overlay1
    );

    app.palette_selected = clear_index;
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer();
    assert_eq!(
        buffer.cell((command_column, row)).unwrap().fg,
        app.theme.palette.base
    );
    assert_eq!(
        buffer.cell((tag_column, row)).unwrap().fg,
        app.theme.palette.base
    );
    assert_eq!(
        buffer.cell((command_column, row)).unwrap().bg,
        app.theme.palette.lavender
    );
}

#[test]
fn escape_cancels_the_palette_then_clears_an_active_kind_filter() {
    let mut app = app();
    let area = Rect::new(0, 0, 100, 20);
    app.kind_filter = Some(resource_kind("", "Child"));
    app.handle_key(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE), area);
    app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE), area);
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), area);

    assert_eq!(app.mode, InputMode::Normal);
    assert_eq!(app.kind_filter, Some(resource_kind("", "Child")));

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), area);
    assert!(app.kind_filter.is_none());
    assert_eq!(app.visible().len(), 2);
}

#[test]
fn command_palette_renders_over_the_resource_view() {
    let mut app = app();
    app.open_palette();
    let backend = TestBackend::new(80, 16);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|frame| render(frame, &app)).unwrap();

    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("commands & resources"));
    assert!(rendered.contains("Child"));
    assert!(rendered.contains("Root"));
    assert!(rendered.contains(":"));

    let popup = palette_area(Rect::new(0, 0, 80, 16), 2);
    let lower_border = resource_tree_area(Rect::new(0, 0, 80, 16))
        .unwrap()
        .bottom()
        .saturating_sub(1);
    assert_eq!(popup.x, 1);
    assert!(usize::from(popup.width.saturating_sub(2)) >= PALETTE_TITLE.width());
    assert_eq!(
        lower_border.saturating_sub(popup.bottom().saturating_sub(1)),
        1
    );
    let selected_row = popup.y + 1;
    let buffer = terminal.backend().buffer();
    assert_eq!(
        buffer.cell((popup.x + 3, selected_row)).unwrap().symbol(),
        "C"
    );
    assert_eq!(
        buffer.cell((popup.x + 3, selected_row)).unwrap().bg,
        app.theme.palette.lavender
    );
    assert_eq!(
        buffer.cell((popup.right() - 2, selected_row)).unwrap().bg,
        app.theme.palette.lavender
    );
}

#[test]
fn status_shortcut_opens_selected_resource_status() {
    let mut app = app();
    app.apply_snapshot(
            Snapshot::parse(
                br#"{"object":{"apiVersion":"example.io/v1","kind":"Root","metadata":{"name":"root"},"status":{"conditions":[{"type":"Ready","status":"True"}],"replicas":2}}}"#,
            )
            .unwrap(),
        );
    let area = Rect::new(0, 0, 100, 20);

    let action = app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE), area);

    assert!(matches!(action, UiAction::None));
    let Some(Modal::Text {
        title,
        content,
        kind,
        ..
    }) = &app.modal
    else {
        panic!("expected status text modal");
    };
    assert_eq!(title, "Status: Root.example.io/root");
    assert_eq!(*kind, ContentKind::Yaml);
    assert!(content.contains("conditions:"));
    assert!(content.contains("type: Ready"));
    assert!(content.contains("replicas: 2"));
    assert!(!content.contains("apiVersion"));
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
    assert!(rendered.contains("r:retry  q:quit"));
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
fn text_selection_is_visible_on_selected_row() {
    let mut app = app();
    let tree_area = resource_tree_area(Rect::new(0, 0, 100, 16)).unwrap();
    let rendered = rendered_tree(&app, tree_area).unwrap();
    let root_start = rendered.lines[0].len() + 1;
    app.tree_selection = Some(TreeSelection {
        text: TextSelection {
            anchor: SelectionPoint {
                start: root_start,
                end: root_start + 1,
            },
            focus: SelectionPoint {
                start: root_start + 3,
                end: root_start + 4,
            },
            dragged: true,
        },
        content: rendered.content,
        area: tree_area,
        selecting: false,
    });

    let backend = TestBackend::new(100, 16);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer();

    assert_eq!(buffer.cell((1, 3)).unwrap().bg, app.theme.palette.yellow);
    assert_eq!(buffer.cell((5, 3)).unwrap().bg, app.theme.palette.lavender);
}

#[test]
fn inspection_content_views_use_the_full_terminal() {
    let area = Rect::new(0, 0, 100, 40);
    assert_eq!(content_modal_area(area, ContentKind::Yaml), area);
    assert_eq!(content_modal_area(area, ContentKind::Describe), area);
    assert_eq!(content_modal_area(area, ContentKind::Events), area);
}

#[test]
fn mutation_error_view_is_large_and_centered() {
    let area = Rect::new(0, 0, 100, 40);

    assert_eq!(
        content_modal_area(area, ContentKind::Error),
        Rect::new(5, 4, 90, 32)
    );
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
        terminal.backend().buffer().cell((0, 0)).unwrap().symbol(),
        "╭"
    );
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
fn content_syntax_colors_are_distinct_from_panel_titles() {
    let theme = app().theme;
    let yaml = styled_content_line("kind: Widget", ContentKind::Yaml, "", &theme);
    let yaml_heading = styled_content_line("metadata:", ContentKind::Yaml, "", &theme);
    let describe_heading = styled_content_line("Containers:", ContentKind::Describe, "", &theme);

    assert_eq!(theme.title().fg, Some(theme.palette.teal));
    assert_eq!(yaml.spans[0].style.fg, Some(theme.palette.sky));
    assert_ne!(yaml.spans[0].style.fg, theme.title().fg);
    assert_eq!(yaml_heading.style.fg, Some(theme.palette.mauve));
    assert_eq!(describe_heading.style.fg, Some(theme.palette.mauve));
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
    assert!(rendered.contains("c:Change propagation"), "{rendered}");
    assert!(rendered.contains("Enter:Delete"));
    assert!(!rendered.contains("UID"));
    assert!(!rendered.contains("must-not-be-rendered"));
    assert_eq!(
        terminal.backend().buffer().cell((10, 4)).unwrap().symbol(),
        "╭"
    );
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
    assert!(rendered.contains("Space:Toggle"), "{rendered}");
    assert!(rendered.contains("Enter:Remove"), "{rendered}");
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
    assert!(rendered.contains("Enter/Space:expand/collapse"));
    assert!(rendered.contains("ctrl-d:delete"));
    assert!(rendered.contains("e:edit"));
    assert!(rendered.contains("::command"));
    assert!(!rendered.contains("j/k:move"));
}

#[test]
fn help_modal_scrolls_to_actions_on_a_standard_terminal() {
    let mut app = app();
    app.mode = InputMode::Help;
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|frame| render(frame, &app)).unwrap();

    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("Navigation"));
    assert!(rendered.contains("Discovery"));
    assert!(!rendered.contains("Actions"));
    assert!(rendered.contains("j/k:scroll"));

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
    assert!(rendered.contains("Esc/q/?:close"));
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
        " drag:copy  j/k or ↑/↓:vertical  h/l or ←/→:horizontal  w:wrap  /:find  n/N:matches  Esc:close"
    );
    assert_eq!(
        content_modal_footer(ContentKind::Yaml, true),
        " drag:copy  j/k or ↑/↓:vertical  w:unwrap  /:find  n/N:matches  Esc:close"
    );
    assert_eq!(
        content_modal_footer(ContentKind::Events, true),
        " drag:copy  j/k or ↑/↓:vertical  /:find  n/N:matches  Esc:close"
    );
    assert_eq!(
        content_modal_footer(ContentKind::Error, true),
        " drag:copy  j/k or ↑/↓:vertical  /:find  n/N:matches  Esc:close"
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
fn mutation_error_modal_wraps_long_lines_and_uses_danger_border() {
    let modal = Modal::Text {
        title: "Delete (Foreground) failed".into(),
        content:
            "Resource: Widget/example\n\nadmission webhook rejected abcdefghijklmnopqrstuvwxyz"
                .into(),
        kind: ContentKind::Error,
        wrapped: true,
        vertical_scroll: 0,
        horizontal_scroll: 0,
        query: String::new(),
        search_input: None,
        selection: None,
    };
    let backend = TestBackend::new(24, 16);
    let mut terminal = Terminal::new(backend).unwrap();
    let theme = app().theme;

    terminal
        .draw(|frame| {
            let area = content_modal_area(frame.area(), ContentKind::Error);
            render_modal(frame, area, &modal, &theme);
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    let area = content_modal_area(Rect::new(0, 0, 24, 16), ContentKind::Error);
    assert_eq!(buffer.cell((area.x, area.y)).unwrap().fg, theme.palette.red);
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
        ContentKind::Error,
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
            copied_mouse_text(
                app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 0), area)
            ),
            None
        );
        assert_eq!(
            copied_mouse_text(
                app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 3), area)
            ),
            None
        );
        assert_eq!(
            copied_mouse_text(
                app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 3), area)
            ),
            Some("kind".into())
        );
        assert!(matches!(
            &app.modal,
            Some(Modal::Text {
                selection: Some(_),
                ..
            })
        ));

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 0), area);
        app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 0), area);
        assert!(matches!(
            &app.modal,
            Some(Modal::Text {
                selection: None,
                ..
            })
        ));
    }
}

#[test]
fn content_mouse_click_does_not_copy_text() {
    let mut app = app();
    app.modal = Some(Modal::Text {
        title: "YAML".into(),
        content: "kind: Widget".into(),
        kind: ContentKind::Yaml,
        wrapped: true,
        vertical_scroll: 0,
        horizontal_scroll: 0,
        query: String::new(),
        search_input: None,
        selection: None,
    });
    let area = Rect::new(0, 0, 40, 10);
    let body = content_modal_body(area, ContentKind::Yaml);
    let mouse = |kind| MouseEvent {
        kind,
        column: body.x,
        row: body.y,
        modifiers: KeyModifiers::NONE,
    };

    assert_eq!(
        copied_mouse_text(app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left)), area)),
        None
    );
    assert_eq!(
        copied_mouse_text(app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left)), area)),
        None
    );
}

#[test]
fn tree_mouse_drag_copies_displayed_text() {
    let mut app = app();
    let area = Rect::new(0, 0, 80, 12);
    let tree_area = resource_tree_area(area).unwrap();
    let body = Block::default().borders(Borders::ALL).inner(tree_area);
    let mouse = |kind, column| MouseEvent {
        kind,
        column: body.x + column,
        row: body.y + 1,
        modifiers: KeyModifiers::NONE,
    };

    assert_eq!(
        copied_mouse_text(
            app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2), area)
        ),
        None
    );
    assert_eq!(
        copied_mouse_text(
            app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 5), area)
        ),
        None
    );
    assert_eq!(
        copied_mouse_text(app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 5), area)),
        Some("Root".into())
    );
    assert!(app.tree_selection.is_some());
}

#[test]
fn tree_refresh_does_not_interrupt_mouse_selection() {
    for changed in [false, true] {
        let mut app = app();
        let area = Rect::new(0, 0, 80, 12);
        let tree_area = resource_tree_area(area).unwrap();
        let body = Block::default().borders(Borders::ALL).inner(tree_area);
        let original = rendered_tree(&app, tree_area).unwrap().content;
        let mouse = |kind, column| MouseEvent {
            kind,
            column: body.x + column,
            row: body.y + 1,
            modifiers: KeyModifiers::NONE,
        };
        let snapshot = app.snapshot.as_ref().unwrap().as_ref().clone();

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2), area);
        app.apply_snapshot(snapshot.clone());
        assert!(app.tree_selection.is_some());
        app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 5), area);

        let latest = if changed {
            Snapshot::parse(
                br#"{"object":{"apiVersion":"v1","kind":"Root","metadata":{"name":"renamed"}}}"#,
            )
            .unwrap()
        } else {
            snapshot
        };
        let latest_count = latest.nodes.len();
        app.apply_snapshot(latest);
        assert_eq!(rendered_tree(&app, tree_area).unwrap().content, original);
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        assert_eq!(
            terminal
                .backend()
                .buffer()
                .cell((body.x + 2, body.y + 1))
                .unwrap()
                .bg,
            app.theme.palette.yellow
        );

        assert_eq!(
            copied_mouse_text(
                app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 5), area)
            ),
            Some("Root".into())
        );
        assert!(app.deferred_snapshot.is_none());
        assert_eq!(app.snapshot.as_ref().unwrap().nodes.len(), latest_count);
        assert_eq!(app.tree_selection.is_some(), !changed);
    }
}

#[test]
fn completed_tree_selection_survives_unchanged_refresh() {
    let mut app = app();
    let area = Rect::new(0, 0, 80, 12);
    let body = Block::default()
        .borders(Borders::ALL)
        .inner(resource_tree_area(area).unwrap());
    let mouse = |kind, column| MouseEvent {
        kind,
        column: body.x + column,
        row: body.y + 1,
        modifiers: KeyModifiers::NONE,
    };
    app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2), area);
    app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 5), area);
    app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 5), area);
    let selection = app.tree_selection.clone();

    app.apply_snapshot(app.snapshot.as_ref().unwrap().as_ref().clone());

    assert_eq!(app.tree_selection, selection);
    assert!(app.deferred_snapshot.is_none());
}

#[test]
fn tree_mouse_click_selects_resource_row_without_copying() {
    let mut app = app();
    let area = Rect::new(0, 0, 80, 12);
    let tree_area = resource_tree_area(area).unwrap();
    let body = Block::default().borders(Borders::ALL).inner(tree_area);
    let mouse = |kind| MouseEvent {
        kind,
        column: body.x,
        row: body.y + 2,
        modifiers: KeyModifiers::NONE,
    };

    assert_eq!(app.selected_visible, 0);
    assert!(
        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left)), area)
            .is_none()
    );
    assert!(
        app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left)), area)
            .is_none()
    );
    assert_eq!(app.selected_visible, 1);
    assert_eq!(app.selected_node().unwrap().identity.kind, "Child");
    assert!(app.tree_selection.is_none());
}

#[test]
fn tree_mouse_double_click_opens_yaml_for_clicked_resource() {
    let mut app = app();
    let area = Rect::new(0, 0, 80, 12);
    let tree_area = resource_tree_area(area).unwrap();
    let body = Block::default().borders(Borders::ALL).inner(tree_area);
    let mouse = |kind| MouseEvent {
        kind,
        column: body.x,
        row: body.y + 2,
        modifiers: KeyModifiers::NONE,
    };

    for _ in 0..2 {
        assert!(
            app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left)), area)
                .is_none()
        );
        let action = app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left)), area);
        if let Some(MouseAction::Action(UiAction::Yaml(target))) = action {
            assert_eq!(target.identity.kind, "Child");
            assert_eq!(target.identity.name, "child");
            return;
        }
    }
    panic!("second click did not open YAML");
}

#[test]
fn tree_right_click_opens_context_menu_for_clicked_resource() {
    let mut app = app();
    let area = Rect::new(0, 0, 80, 12);
    let tree_area = resource_tree_area(area).unwrap();
    let body = Block::default().borders(Borders::ALL).inner(tree_area);
    let mouse = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right),
        column: body.x + 4,
        row: body.y + 2,
        modifiers: KeyModifiers::NONE,
    };

    assert!(app.handle_mouse(mouse, area).is_none());
    let Some(Modal::ContextMenu {
        target,
        column,
        row,
        ..
    }) = &app.modal
    else {
        panic!("expected context menu");
    };
    assert_eq!(target.identity.kind, "Child");
    assert_eq!((*column, *row), (mouse.column, mouse.row));
    assert_eq!(app.selected_visible, 1);
}

#[test]
fn context_menu_options_dispatch_matching_actions() {
    let area = Rect::new(0, 0, 80, 12);
    for (item, expected) in [(0, "yaml"), (1, "edit"), (3, "events"), (4, "describe")] {
        let mut app = app();
        app.modal = Some(Modal::ContextMenu {
            target: app.selected_target().unwrap(),
            column: 10,
            row: 3,
            pressed: None,
        });
        let menu = context_menu_area(area, 10, 3);
        let inner = Block::default().borders(Borders::ALL).inner(menu);
        let click = |kind| MouseEvent {
            kind,
            column: inner.x,
            row: inner.y + item as u16,
            modifiers: KeyModifiers::NONE,
        };
        assert!(
            app.handle_mouse(click(MouseEventKind::Down(MouseButton::Left)), area)
                .is_none()
        );
        let action = app.handle_mouse(click(MouseEventKind::Up(MouseButton::Left)), area);

        let (actual, target) = match action {
            Some(MouseAction::Action(UiAction::Yaml(target))) => ("yaml", target),
            Some(MouseAction::Action(UiAction::Edit(target))) => ("edit", target),
            Some(MouseAction::Action(UiAction::Events(target))) => ("events", target),
            Some(MouseAction::Action(UiAction::Describe(target))) => ("describe", target),
            _ => panic!("expected context menu action"),
        };
        assert_eq!(actual, expected);
        assert_eq!(target.identity.kind, "Root");
        assert!(app.modal.is_none());
    }
}

#[test]
fn context_menu_status_opens_the_status_modal() {
    let mut app = app();
    let area = Rect::new(0, 0, 80, 12);
    app.modal = Some(Modal::ContextMenu {
        target: app.selected_target().unwrap(),
        column: 10,
        row: 3,
        pressed: None,
    });
    let menu = context_menu_area(area, 10, 3);
    let inner = Block::default().borders(Borders::ALL).inner(menu);
    let click = |kind| MouseEvent {
        kind,
        column: inner.x,
        row: inner.y + 2,
        modifiers: KeyModifiers::NONE,
    };

    assert!(
        app.handle_mouse(click(MouseEventKind::Down(MouseButton::Left)), area)
            .is_none()
    );
    assert!(
        app.handle_mouse(click(MouseEventKind::Up(MouseButton::Left)), area)
            .is_none()
    );
    let Some(Modal::Text { title, content, .. }) = &app.modal else {
        panic!("expected status modal");
    };
    assert_eq!(title, "Status: Root/root");
    assert_eq!(content, "No status reported.\n");
}

#[test]
fn clicking_outside_context_menu_only_dismisses_it() {
    let mut app = app();
    let area = Rect::new(0, 0, 80, 12);
    app.modal = Some(Modal::ContextMenu {
        target: app.selected_target().unwrap(),
        column: 20,
        row: 3,
        pressed: None,
    });
    let outside = |kind| MouseEvent {
        kind,
        column: 0,
        row: 0,
        modifiers: KeyModifiers::NONE,
    };

    assert!(
        app.handle_mouse(outside(MouseEventKind::Down(MouseButton::Left)), area)
            .is_none()
    );
    assert!(app.modal.is_none());
    assert!(!app.quit);
    assert!(
        app.handle_mouse(outside(MouseEventKind::Up(MouseButton::Left)), area)
            .is_none()
    );
    assert!(!app.quit);
}

#[test]
fn context_menu_is_placed_clear_of_clicked_row() {
    let area = Rect::new(0, 0, 80, 24);

    let below = context_menu_area(area, 10, 5);
    assert!(below.y > 5);

    let above = context_menu_area(area, 10, 22);
    assert!(above.bottom() <= 22);
}

#[test]
fn context_menu_uses_normal_text_and_selected_option_styles() {
    let theme = app().theme;

    let normal = context_menu_item_style(&theme, 0, None);
    assert_eq!(normal.fg, Some(theme.palette.text));
    assert_ne!(normal.fg, theme.title().fg);
    assert_eq!(
        context_menu_item_style(&theme, 1, Some(1)),
        theme.selected_option()
    );
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
        dragged: true,
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
