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
        state,
        ..
    }) = &app.modal
    else {
        panic!("expected persistent mutation error modal");
    };
    assert_eq!(title, "Delete (Foreground) failed");
    assert!(content.contains(&format!("Resource: {identity}")));
    assert!(content.contains("admission webhook denied the request"));
    assert_eq!(*kind, ContentKind::Error);
    assert!(state.wrapped);

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
fn inspection_failure_opens_error_modal() {
    let mut app = app();

    let succeeded = app.finish_action(
        "Events: Root/root",
        None,
        Err("request was forbidden".into()),
    );

    assert!(!succeeded);
    assert_eq!(app.status, "Events: Root/root failed");
    let Some(Modal::Text {
        title,
        content,
        kind,
        ..
    }) = &app.modal
    else {
        panic!("expected error modal");
    };
    assert_eq!(title, "Events: Root/root failed");
    assert!(content.contains("request was forbidden"));
    assert_eq!(*kind, ContentKind::Error);
}

#[test]
fn enter_and_space_never_toggle_tree_expansion() {
    let mut app = app();
    let area = Rect::new(0, 0, 100, 20);
    for collapsed in [false, true] {
        for key in [KeyCode::Enter, KeyCode::Char(' ')] {
            app.handle_key(KeyEvent::new(key, KeyModifiers::NONE), area);
            assert_eq!(app.visible().len(), if collapsed { 1 } else { 2 });
            assert!(app.modal.is_none());
        }
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), area);
    }
}

#[test]
fn problematic_parent_opens_details_without_changing_expansion() {
    let mut app = app();
    app.apply_snapshot(Snapshot::parse(br#"{"object":{"apiVersion":"v1","kind":"Root","metadata":{"name":"root"}},"error":{"message":"trace failed"},"children":[{"object":{"apiVersion":"v1","kind":"Child","metadata":{"name":"child"}}}]}"#).unwrap());
    let area = Rect::new(0, 0, 100, 20);
    for collapsed in [false, true] {
        let before = app.collapsed.clone();
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE), area);
        assert!(app.modal.is_none());
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), area);
        assert!(matches!(app.modal, Some(Modal::Text { .. })));
        assert_eq!(app.collapsed, before);
        assert_eq!(app.visible().len(), if collapsed { 1 } else { 2 });
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), area);
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), area);
    }
}

#[test]
fn resource_details_support_long_safe_text_scrolling_and_search() {
    let mut app = app();
    let message = format!(
        "{}\nneedle\u{1b}[31m\u{202e}",
        "long diagnostic\n".repeat(60)
    );
    let trace = serde_json::json!({"object":{"apiVersion":"v1","kind":"Root","metadata":{"name":"root"}},"error":{"message":message}});
    app.apply_snapshot(Snapshot::parse(&serde_json::to_vec(&trace).unwrap()).unwrap());
    let area = Rect::new(0, 0, 100, 20);
    let tree = rendered_tree(&app, resource_tree_area(area).unwrap()).unwrap();
    assert!(tree.lines[1].contains("Error"));
    assert!(!tree.lines[1].contains("diagnostic"));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), area);
    app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), area);
    assert!(
        matches!(&app.modal, Some(Modal::Text { content, state, .. }) if state.vertical_scroll > 0 && content.contains("needle\\x1b[31m\\u{202E}") && !content.contains('\u{1b}') && !content.contains('\u{202e}'))
    );
    app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE), area);
    for character in "needle".chars() {
        app.handle_key(
            KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE),
            area,
        );
    }
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), area);
    assert!(
        matches!(&app.modal, Some(Modal::Text { state, .. }) if state.query == "needle" && state.vertical_scroll > 40)
    );
    let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    assert!(terminal.backend().to_string().contains("needle"));
}

#[test]
fn enter_opens_wrapped_problem_details_that_survive_refresh() {
    let mut app = app();
    let trace = serde_json::json!({
        "object":{"apiVersion":"v1","kind":"Root","metadata":{"name":"root"}},
        "children":[{"object":{"apiVersion":"v1","kind":"Child","metadata":{"name":"child"},"status":{"conditions":[{"type":"Ready","status":"False","reason":"Waiting","message":"a long diagnostic"}]}},"error":{"message":"cannot connect"}}]
    });
    app.apply_snapshot(Snapshot::parse(&serde_json::to_vec(&trace).unwrap()).unwrap());
    let area = Rect::new(0, 0, 100, 20);
    app.find = "cannot connect".into();
    app.find_next(false);
    assert_eq!(app.selection.visible_index, 1);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), area);
    let Some(Modal::Text {
        title,
        content,
        kind,
        state,
        ..
    }) = &app.modal
    else {
        panic!("expected resource problem details");
    };
    assert_eq!(title, "Resource details: Child/child");
    assert!(content.contains("Error: cannot connect"));
    assert!(content.contains("Ready=False\nWaiting: a long diagnostic"));
    assert_eq!(*kind, ContentKind::Error);
    assert!(state.wrapped);
    let original = content.clone();
    app.apply_snapshot(
        Snapshot::parse(
            br#"{"object":{"apiVersion":"v1","kind":"Root","metadata":{"name":"root"}}}"#,
        )
        .unwrap(),
    );
    assert!(matches!(&app.modal, Some(Modal::Text { content, .. }) if content == &original));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), area);
    assert!(app.modal.is_none());
}

#[test]
fn uppercase_e_opens_events_and_lowercase_bindings_remain_distinct() {
    let mut app = app();
    let area = Rect::new(0, 0, 100, 20);
    for modifiers in [KeyModifiers::NONE, KeyModifiers::SHIFT] {
        assert!(
            matches!(app.handle_key(KeyEvent::new(KeyCode::Char('E'), modifiers), area), UiAction::Events(target) if target.identity == app.selected_node().unwrap().identity)
        );
    }
    assert!(matches!(
        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE), area),
        UiAction::Edit(_)
    ));
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
    assert_eq!(app.selection.visible_index, 0);
}

#[test]
fn right_expands_a_collapsed_node() {
    let mut app = app();
    let area = Rect::new(0, 0, 100, 20);
    app.collapse_or_parent();

    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), area);

    assert_eq!(app.visible().len(), 2);
    assert_eq!(app.selection.visible_index, 0);
}

#[test]
fn right_selects_the_first_child_of_an_expanded_node() {
    let mut app = app();
    let area = Rect::new(0, 0, 100, 20);

    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), area);

    assert_eq!(app.selection.visible_index, 1);
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
    assert_eq!(app.selection.visible_index, 6);
    assert_eq!(app.resource_scroll, 6);

    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), area);
    assert_eq!(app.selection.visible_index, 5);
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

    app.collapse_or_parent();
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

    app.collapse_or_parent();
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
fn reload_palette_command_applies_configuration_and_preserves_session_state() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    std::fs::write(
        &path,
        r#"
read_only = true
[trace]
program = "custom-trace"
interval_seconds = 12
timeout_seconds = 30
retry_backoff_max_seconds = 2
[ui]
color = "never"
ascii = true
short = true
[skin]
name = "nord"
[[extra_columns.Root]]
name = "NAME"
path = "/metadata/name"
"#,
    )
    .unwrap();
    let mut app = app();
    app.config_path = path;
    app.set_selection(1);
    let selected = app.selection.identity.clone();
    let snapshot = app.snapshot.clone().unwrap();
    app.collapsed.insert(selected.clone().unwrap());
    app.excluded_kinds.insert(resource_kind("", "Unused"));
    app.find = "child".into();
    app.paused = true;
    app.retry_delay = Duration::from_secs(16);
    let area = Rect::new(0, 0, 100, 24);
    for key in ":reload".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE), area);
    }

    let entries = app.palette_entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].action, PaletteAction::ReloadConfig);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), area);

    assert_eq!(app.mode, InputMode::Normal);
    assert!(app.input.is_empty());
    assert_eq!(app.status, "Configuration reloaded");
    assert!(app.config.read_only);
    assert!(app.config.ui.ascii);
    assert!(app.config.ui.short);
    assert_eq!(app.theme.resolved_name, "nord");
    assert!(!app.theme.colors_enabled);
    assert_eq!(app.config.extra_columns["Root"][0].name, "NAME");
    assert_eq!(app.config.trace.program, "custom-trace");
    assert_eq!(app.config.interval(), Duration::from_secs(12));
    assert_eq!(app.config.timeout(), Some(Duration::from_secs(30)));
    assert_eq!(app.retry_delay, Duration::from_secs(2));
    assert_eq!(app.selection.identity, selected);
    assert_eq!(app.selection.visible_index, 1);
    assert!(Arc::ptr_eq(app.snapshot.as_ref().unwrap(), &snapshot));
    assert!(app.collapsed.contains(selected.as_ref().unwrap()));
    assert!(app.excluded_kinds.contains(&resource_kind("", "Unused")));
    assert_eq!(app.find, "child");
    assert!(app.paused);
}

#[test]
fn reload_configuration_preserves_command_line_overrides_and_filters() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    std::fs::write(&path, "[skin]\nname = 'nord'\n").unwrap();
    let cli = Cli::parse_from([
        "xpdelve",
        "--config",
        path.to_str().unwrap(),
        "--readonly",
        "--short",
        "--cmd",
        "custom-trace --json",
        "--watch-interval",
        "8",
        "--no-watch",
        "Root/root",
    ]);
    let config = Config::load(&cli).unwrap();
    let theme = Theme::resolve(&config.skin, config.ui.color).unwrap();
    let mut app = App::new("Root/root".into(), config, &cli, theme);
    app.filter = "child".into();
    app.kind_filter = Some(resource_kind("", "Child"));
    app.health_filter = HealthFilter::Unhealthy;
    app.show_logical_name = false;
    std::fs::write(
        path,
        "read_only = false\n[trace]\nprogram = 'other-trace'\ninterval_seconds = 20\n[ui]\nshort = false\n[skin]\nname = 'nord'\n",
    )
    .unwrap();

    app.reload_config();

    assert_eq!(app.status, "Configuration reloaded");
    assert!(app.config.read_only);
    assert!(app.config.ui.short);
    assert_eq!(app.config.trace.program, "custom-trace");
    assert_eq!(app.config.trace.args, ["--json"]);
    assert_eq!(app.config.interval(), Duration::from_secs(8));
    assert!(app.no_watch);
    assert_eq!(app.filter, "child");
    assert_eq!(app.kind_filter, Some(resource_kind("", "Child")));
    assert_eq!(app.health_filter, HealthFilter::Unhealthy);
    assert!(!app.show_logical_name);
}

#[test]
fn reload_configuration_rejects_invalid_or_missing_files_without_changing_settings() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = app();
    app.config_path = directory.path().join("config.toml");
    for source in [
        "invalid toml",
        "schema_version = 2",
        "[trace]\ninterval_seconds = 0",
        "[ui]\nascii = true\n[skin]\nname = 'unknown-theme'",
        "[[extra_columns.Root]]\nname = 'NAME'\npath = 'invalid'",
    ] {
        std::fs::write(&app.config_path, source).unwrap();
        app.reload_config();
        assert_eq!(app.status, "Could not reload configuration");
        assert!(matches!(app.modal, Some(Modal::Text {
            kind: ContentKind::SmallError,
            ref content,
            ..
        }) if !content.is_empty()));
        assert_eq!(app.config.interval(), Duration::from_secs(5));
        assert!(!app.config.ui.ascii);
        assert_eq!(app.theme.resolved_name, "catppuccin-mocha");
        assert!(app.config.extra_columns.is_empty());
    }
    app.config_path = directory.path().join("missing.toml");
    app.reload_config();
    assert!(matches!(app.modal, Some(Modal::Text {
        kind: ContentKind::SmallError,
        ref content,
        ..
    }) if content.contains("configuration file does not exist")));
    assert_eq!(app.theme.resolved_name, "catppuccin-mocha");
}

#[test]
fn reload_failure_modal_is_compact_error_styled_and_dismissible() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = app();
    app.config_path = directory.path().join("missing.toml");
    app.mode = InputMode::Command;
    app.input = "reload".into();
    let area = Rect::new(0, 0, 100, 30);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), area);

    assert_eq!(app.mode, InputMode::Normal);
    let popup = content_modal_area(area, ContentKind::SmallError);
    assert_eq!(popup, Rect::new(14, 9, 72, 12));
    let backend = TestBackend::new(area.width, area.height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer();
    assert_eq!(
        buffer.cell((popup.x, popup.y)).unwrap().fg,
        app.theme.palette.red
    );
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("Configuration reload failed"));
    assert!(rendered.contains("configuration file does not exist"));
    assert!(rendered.contains("Esc/q:close"));

    app.apply_resource_not_found();
    assert!(matches!(
        app.modal,
        Some(Modal::Text {
            kind: ContentKind::SmallError,
            ..
        })
    ));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), area);
    assert!(app.modal.is_none());
    app.reload_config();
    app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE), area);
    assert!(app.modal.is_none());
    assert!(!app.quit);
}

#[test]
fn reload_error_modal_wraps_scrolls_and_fits_small_terminals() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = app();
    app.config_path = directory.path().join("config.toml");
    std::fs::write(&app.config_path, "unknown_field = true\n".repeat(30)).unwrap();
    app.reload_config();
    let area = Rect::new(0, 0, 40, 10);
    assert_eq!(
        content_modal_area(area, ContentKind::SmallError),
        Rect::new(1, 1, 38, 8)
    );
    let Some(Modal::Text { content, state, .. }) = &app.modal else {
        panic!("expected error modal");
    };
    assert!(state.wrapped);
    assert!(!content.contains('\u{1b}'));
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), area);
    assert!(matches!(
        app.modal,
        Some(Modal::Text { ref state, .. }) if state.vertical_scroll == 1
    ));
    let backend = TestBackend::new(area.width, area.height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
}

#[tokio::test]
async fn configuration_reload_updates_the_refresh_timer_only_when_interval_changes() {
    let mut refresh = time::interval(Duration::from_secs(5));
    super::runtime::update_refresh_interval(&mut refresh, Duration::from_secs(12));
    assert_eq!(refresh.period(), Duration::from_secs(12));
    assert_eq!(
        refresh.missed_tick_behavior(),
        time::MissedTickBehavior::Skip
    );
    // An unchanged interval must not postpone the existing refresh deadline.
    refresh.reset_at(time::Instant::now());
    super::runtime::update_refresh_interval(&mut refresh, Duration::from_secs(12));
    assert!(
        time::timeout(Duration::from_millis(100), refresh.tick())
            .await
            .is_ok()
    );
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
    assert_eq!(app.selection.visible_index, 0);
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
fn paused_resources_show_paused_status_and_theme_orange_without_name_suffix() {
    for name in crate::theme::BUILTIN_NAMES {
        for color in [
            crate::config::ColorMode::Always,
            crate::config::ColorMode::Never,
        ] {
            let mut app = app_with_theme(name, color);
            app.apply_snapshot(
                Snapshot::parse(
                    br#"{"object":{"apiVersion":"v1","kind":"Root","metadata":{"name":"root"}},"children":[{"object":{"apiVersion":"v1","kind":"Child","metadata":{"name":"child","annotations":{"crossplane.io/paused":"true"}},"status":{"conditions":[{"type":"Synced","status":"False","reason":"ReconcilePaused"}]}},"error":{"message":"reconciliation paused"}}]}"#,
                )
                .unwrap(),
            );
            let snapshot = app.snapshot.as_ref().unwrap();
            let child = &snapshot.nodes[1];
            assert!(child.paused);
            assert_eq!(child.health, crate::model::Health::Unhealthy);
            assert_eq!(child.status, "Error");
            assert!(object_cell(snapshot, child, false).ends_with("Child/child"));

            let area = Rect::new(0, 0, 100, 12);
            let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
            terminal
                .draw(|frame| render_tree(frame, area, &app))
                .unwrap();
            let rendered = terminal.backend().to_string();
            assert!(rendered.contains("Paused"));
            assert!(!rendered.contains("(paused)"));
            assert!(!rendered.contains("Error"));
            let buffer = terminal.backend().buffer();
            // The first resource is selected; the paused child is the next row.
            let child_row = 3;
            let expected = if app.theme.colors_enabled {
                app.theme.palette.peach
            } else {
                Color::Reset
            };
            assert_eq!(buffer.cell((1, child_row)).unwrap().fg, expected);
            let status_column = rendered_tree(&app, area).unwrap().lines[0]
                .find("STATUS")
                .unwrap();
            // Find the status cell by display width rather than UTF-8 byte offset.
            let status_column =
                rendered_tree(&app, area).unwrap().lines[0][..status_column].width();
            assert_eq!(
                buffer
                    .cell((1 + status_column as u16, child_row))
                    .unwrap()
                    .fg,
                expected
            );

            app.selection.visible_index = 1;
            terminal
                .draw(|frame| render_tree(frame, area, &app))
                .unwrap();
            let selected = terminal.backend().buffer().cell((1, child_row)).unwrap();
            if app.theme.colors_enabled {
                assert_eq!(selected.fg, app.theme.palette.base);
                assert_eq!(selected.bg, app.theme.palette.lavender);
            } else {
                assert!(selected.modifier.contains(Modifier::REVERSED));
            }
        }
    }
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
        state: TextModalState::new(ContentKind::Yaml),
        selection: None,
    });

    app.apply_resource_not_found();

    assert!(app.resource_missing);
    assert!(app.snapshot.is_none());
    assert!(app.selection.identity.is_none());
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

fn app_with_extra_columns() -> App {
    let mut app = app();
    app.config.extra_columns = toml::from_str::<Config>(
        r#"
        [[extra_columns.Root]]
        name = "VALUE"
        path = "/status/value"
        width = 12
        [[extra_columns.Root]]
        name = "FLAG"
        path = "/status/enabled"
        [[extra_columns.Child]]
        name = "value"
        path = "/metadata/name"
        width = 8
        [[extra_columns.Child]]
        name = "CHILD ONLY"
        path = "/metadata/name"
        "#,
    )
    .unwrap()
    .extra_columns;
    app.apply_snapshot(Snapshot::parse(br#"{"object":{"apiVersion":"v1","kind":"Root","metadata":{"name":"root"},"status":{"value":"workspace","enabled":true}},"children":[{"object":{"apiVersion":"v1","kind":"Child","metadata":{"name":"child"}}}]}"#).unwrap());
    app
}

#[test]
fn extra_columns_union_is_aligned_before_status_in_both_table_modes() {
    let mut app = app_with_extra_columns();
    for short in [false, true] {
        app.config.ui.short = short;
        let area = Rect::new(0, 0, 240, 20);
        let rendered = rendered_tree(&app, resource_tree_area(area).unwrap()).unwrap();
        let header = &rendered.lines[0];
        let value = header[..header.find("VALUE").unwrap()].width();
        let child = header[..header.find("CHILD ONLY").unwrap()].width();
        let flag = header[..header.find("FLAG").unwrap()].width();
        let status = header[..header.find("STATUS").unwrap()].width();
        assert!(value < child && child < flag && flag < status);
        assert_eq!(child - value, 14);
        assert_eq!(
            horizontal_slice(&rendered.lines[1], value, 12).trim(),
            "workspace"
        );
        assert_eq!(
            horizontal_slice(&rendered.lines[2], value, 12).trim(),
            "child"
        );
        assert_eq!(horizontal_slice(&rendered.lines[1], child, 10).trim(), "-");
        assert_eq!(horizontal_slice(&rendered.lines[2], flag, 4).trim(), "-");
        assert_eq!(header.contains("LAST"), !short);
        let plan =
            tree_table_plan(&app, resource_tree_area(area).unwrap(), &app.visible()).unwrap();
        assert_eq!(
            plan.header().width() + 1,
            plan.horizontal_layout(0).pinned_width
                + plan.horizontal_layout(0).content_width
                + plan.horizontal_layout(0).right_pinned_width
        );
        let backend = TestBackend::new(240, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        assert!(terminal.backend().to_string().contains("workspace"));
    }
}

#[test]
fn extra_columns_follow_collapse_kind_filter_and_refresh() {
    let mut app = app_with_extra_columns();
    let area = Rect::new(0, 0, 240, 20);
    let tree = resource_tree_area(area).unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), area);
    assert!(!rendered_tree(&app, tree).unwrap().lines[0].contains("CHILD ONLY"));
    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), area);
    assert!(rendered_tree(&app, tree).unwrap().lines[0].contains("CHILD ONLY"));
    for key in [':', 'c', 'h'] {
        app.handle_key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE), area);
    }
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), area);
    let filtered = rendered_tree(&app, tree).unwrap();
    assert!(filtered.lines[0].contains("CHILD ONLY"));
    assert!(!filtered.lines[0].contains("FLAG"));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), area);
    app.apply_snapshot(Snapshot::parse(br#"{"object":{"apiVersion":"v1","kind":"Root","metadata":{"name":"root"},"status":{"value":"updated","enabled":false}}}"#).unwrap());
    let refreshed = rendered_tree(&app, tree).unwrap();
    assert!(refreshed.lines[1].contains("updated"));
    assert!(refreshed.lines[1].contains("false"));
    assert!(!refreshed.lines[0].contains("CHILD ONLY"));
}

#[test]
fn extra_columns_scroll_with_the_table_while_objects_remain_pinned() {
    let mut app = app_with_extra_columns();
    let area = Rect::new(0, 0, 60, 20);
    let tree = resource_tree_area(area).unwrap();
    let before = rendered_tree(&app, tree).unwrap();
    for _ in 0..100 {
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE), area);
    }
    let after = rendered_tree(&app, tree).unwrap();
    assert_eq!(after.horizontal.offset, after.horizontal.max_offset());
    assert!(after.lines[0].contains("STATUS"));
    assert!(after.lines[0].contains("FLAG"));
    assert_eq!(
        horizontal_slice(&before.lines[1], 0, before.horizontal.pinned_width),
        horizontal_slice(&after.lines[1], 0, after.horizontal.pinned_width)
    );
}

#[test]
fn extra_columns_work_in_package_tables() {
    let mut app = app();
    app.config.extra_columns = toml::from_str::<Config>(
        "[[extra_columns.\"Provider.pkg.crossplane.io\"]]\nname='PACKAGE'\npath='/spec/package'",
    )
    .unwrap()
    .extra_columns;
    app.apply_snapshot(Snapshot::parse(br#"{"object":{"apiVersion":"pkg.crossplane.io/v1","kind":"Provider","metadata":{"name":"provider"},"spec":{"package":"example:v1"}}}"#).unwrap());
    for short in [false, true] {
        app.config.ui.short = short;
        let area = Rect::new(0, 0, 240, 20);
        let rendered = rendered_tree(&app, resource_tree_area(area).unwrap()).unwrap();
        let header = &rendered.lines[0];
        assert!(header.find("PACKAGE").unwrap() < header.find("STATUS").unwrap());
        assert!(rendered.lines[1].contains("example:v1"));
        assert!(header.contains("INSTALLED"));
        assert!(header.contains("HEALTHY"));
        assert!(!header.contains("GROUP"));
        let plan =
            tree_table_plan(&app, resource_tree_area(area).unwrap(), &app.visible()).unwrap();
        assert_eq!(
            plan.header().width() + 1,
            plan.horizontal_layout(0).pinned_width
                + plan.horizontal_layout(0).content_width
                + plan.horizontal_layout(0).right_pinned_width
        );
    }
}

#[test]
fn extra_columns_follow_exclusions_and_do_not_change_when_scrolling_vertically() {
    let mut app = app_with_extra_columns();
    let mut trace = serde_json::json!({
        "object": {"apiVersion":"v1", "kind":"Root", "metadata":{"name":"root"}},
        "children": []
    });
    let children = trace["children"].as_array_mut().unwrap();
    for index in 0..15 {
        children.push(serde_json::json!({"object":{"apiVersion":"v1", "kind":"Other", "metadata":{"name":format!("other-{index}")}}}));
    }
    children.push(serde_json::json!({"object":{"apiVersion":"v1", "kind":"Child", "metadata":{"name":"child"}}}));
    app.apply_snapshot(Snapshot::parse(&serde_json::to_vec(&trace).unwrap()).unwrap());
    let area = Rect::new(0, 0, 240, 12);
    let tree = resource_tree_area(area).unwrap();
    let before = rendered_tree(&app, tree).unwrap();
    assert!(before.lines[0].contains("CHILD ONLY"));
    assert!(
        !before
            .lines
            .iter()
            .skip(1)
            .any(|line| line.contains("Child/child"))
    );
    for _ in 0..20 {
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE), area);
    }
    let after = rendered_tree(&app, tree).unwrap();
    assert_eq!(before.lines[0], after.lines[0]);
    assert!(after.lines.iter().any(|line| line.contains("Child/child")));
    for key in ":exclude".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE), area);
    }
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), area);
    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE), area);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), area);
    assert!(app.excluded_kinds.contains(&resource_kind("", "Child")));
    let excluded = rendered_tree(&app, tree).unwrap();
    assert!(!excluded.lines[0].contains("CHILD ONLY"));
    assert!(excluded.lines[0].contains("FLAG"));
}

#[test]
fn extra_columns_truncate_unicode_by_display_width() {
    let mut app = app();
    app.config.extra_columns = toml::from_str::<Config>("[[extra_columns.Root]]\nname='文字'\npath='/status/value'\nwidth=6\n[[extra_columns.Root]]\nname='NEXT'\npath='/metadata/name'").unwrap().extra_columns;
    app.apply_snapshot(Snapshot::parse("{\"object\":{\"apiVersion\":\"v1\",\"kind\":\"Root\",\"metadata\":{\"name\":\"root\"},\"status\":{\"value\":\"界界界界\"}}}".as_bytes()).unwrap());
    let area = Rect::new(0, 0, 240, 20);
    let rendered = rendered_tree(&app, resource_tree_area(area).unwrap()).unwrap();
    let header = &rendered.lines[0];
    let value = header[..header.find("文字").unwrap()].width();
    let next = header[..header.find("NEXT").unwrap()].width();
    assert_eq!(next - value, 8);
    assert_eq!(horizontal_slice(&rendered.lines[1], value, 6), "界界… ");
    assert_eq!(horizontal_slice(&rendered.lines[1], next, 4), "root");
}

#[test]
fn unmatched_extra_columns_preserve_existing_table_rendering() {
    let mut app = app();
    let area = Rect::new(0, 0, 240, 20);
    let tree = resource_tree_area(area).unwrap();
    let before = rendered_tree(&app, tree).unwrap();
    app.config.extra_columns = toml::from_str::<Config>(
        "[[extra_columns.\"Widget.example.org\"]]\nname='VALUE'\npath='/status/value'",
    )
    .unwrap()
    .extra_columns;
    let after = rendered_tree(&app, tree).unwrap();
    assert_eq!(before.lines, after.lines);
    assert_eq!(
        before.horizontal.content_width,
        after.horizontal.content_width
    );
}

#[test]
fn logical_name_is_the_first_scrollable_column_in_all_table_modes() {
    for (api_version, kind) in [
        ("example.io/v1", "Root"),
        ("pkg.crossplane.io/v1", "Provider"),
    ] {
        let mut app = app();
        app.show_logical_name = true;
        let logical_name = "composed-界界-resource";
        let trace = serde_json::json!({
            "object": {
                "apiVersion": api_version,
                "kind": kind,
                "metadata": {"name": "root"}
            },
            "children": [{"object": {
                "apiVersion": "v1",
                "kind": "Child",
                "metadata": {
                    "name": "child",
                    "annotations": {"crossplane.io/composition-resource-name": logical_name}
                }
            }}]
        });
        app.apply_snapshot(Snapshot::parse(&serde_json::to_vec(&trace).unwrap()).unwrap());
        for short in [false, true] {
            app.config.ui.short = short;
            for ascii in [false, true] {
                app.config.ui.ascii = ascii;
                let tree = resource_tree_area(Rect::new(0, 0, 240, 16)).unwrap();
                let rendered = rendered_tree(&app, tree).unwrap();
                let start = rendered.horizontal.pinned_width;
                assert_eq!(
                    horizontal_slice(&rendered.lines[0], start, "LOGICAL NAME".len()),
                    "LOGICAL NAME"
                );
                assert_eq!(
                    horizontal_slice(&rendered.lines[1], start, logical_name.width()).trim(),
                    "-"
                );
                assert_eq!(
                    horizontal_slice(&rendered.lines[2], start, logical_name.width()),
                    logical_name
                );
                let plan = tree_table_plan(&app, tree, &app.visible()).unwrap();
                assert_eq!(
                    plan.header().width() + 1,
                    rendered.horizontal.pinned_width
                        + rendered.horizontal.content_width
                        + rendered.horizontal.right_pinned_width
                );
                let narrow = resource_tree_area(Rect::new(0, 0, 50, 16)).unwrap();
                let before = rendered_tree(&app, narrow).unwrap();
                app.resource_horizontal_scroll = 4;
                let after = rendered_tree(&app, narrow).unwrap();
                assert_eq!(
                    horizontal_slice(&after.lines[0], after.horizontal.pinned_width, 7),
                    "CAL NAM"
                );
                assert_eq!(
                    horizontal_slice(&before.lines[2], 0, before.horizontal.pinned_width),
                    horizontal_slice(&after.lines[2], 0, after.horizontal.pinned_width)
                );
                app.resource_horizontal_scroll = 0;
            }
        }
    }
}

#[test]
fn logical_name_visibility_uses_config_and_reload_applies_changed_values() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    std::fs::write(&path, "[ui]\nshow_logical_name = false\n").unwrap();
    let cli = Cli::parse_from(["xpdelve", "--config", path.to_str().unwrap(), "Root/root"]);
    let config = Config::load(&cli).unwrap();
    let theme = Theme::resolve(&config.skin, config.ui.color).unwrap();
    let mut app = App::new("Root/root".into(), config, &cli, theme);
    let trace = br#"{"object":{"apiVersion":"v1","kind":"Root","metadata":{"name":"root"}}}"#;
    app.apply_snapshot(Snapshot::parse(trace).unwrap());
    let area = Rect::new(0, 0, 240, 16);
    let tree = resource_tree_area(area).unwrap();
    assert!(!app.show_logical_name);
    assert!(!rendered_tree(&app, tree).unwrap().lines[0].contains("LOGICAL NAME"));

    app.handle_key(KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT), area);
    assert!(app.show_logical_name);
    app.reload_config();
    assert!(app.show_logical_name);
    assert!(!app.config.ui.show_logical_name);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "[ui]\nshow_logical_name = false\n"
    );

    std::fs::write(&path, "[ui]\nshow_logical_name = true\n").unwrap();
    app.reload_config();
    assert!(app.show_logical_name);

    // Reloading a changed setting takes precedence over the session toggle.
    std::fs::write(&path, "[ui]\nshow_logical_name = false\n").unwrap();
    app.reload_config();
    assert!(!app.show_logical_name);
    app.handle_key(KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT), area);
    assert!(app.show_logical_name);
    std::fs::write(&path, "[ui]\nshow_logical_name = true\n").unwrap();
    app.reload_config();
    assert!(app.show_logical_name);

    std::fs::write(&path, "[ui]\nshow_logical_name = false\n").unwrap();
    app.reload_config();
    assert!(!app.show_logical_name);
    std::fs::write(&path, "[ui]\nshort = true\n").unwrap();
    app.reload_config();
    assert!(!app.show_logical_name);
    assert!(!rendered_tree(&app, tree).unwrap().lines[0].contains("LOGICAL NAME"));
}

#[test]
fn logical_name_toggle_removes_the_column_and_preserves_pinned_regions() {
    for (api_version, kind) in [
        ("example.io/v1", "Root"),
        ("pkg.crossplane.io/v1", "Provider"),
    ] {
        for short in [false, true] {
            for ascii in [false, true] {
                let mut app = app();
                app.show_logical_name = true;
                let trace = serde_json::json!({"object": {
                    "apiVersion": api_version, "kind": kind,
                    "metadata": {"name": "root", "annotations": {
                        "crossplane.io/composition-resource-name": "logical-resource"
                    }}
                }});
                app.apply_snapshot(Snapshot::parse(&serde_json::to_vec(&trace).unwrap()).unwrap());
                app.config.ui.short = short;
                app.config.ui.ascii = ascii;
                let area = Rect::new(0, 0, 240, 16);
                let tree = resource_tree_area(area).unwrap();
                let before = rendered_tree(&app, tree).unwrap();
                assert!(app.show_logical_name);
                assert!(before.lines[0].contains("LOGICAL NAME"));
                app.handle_key(KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT), area);
                let hidden = rendered_tree(&app, tree).unwrap();
                assert!(!app.show_logical_name);
                assert!(!hidden.lines[0].contains("LOGICAL NAME"));
                assert!(!hidden.lines[1].contains("logical-resource"));
                assert_eq!(
                    before.horizontal.content_width - hidden.horizontal.content_width,
                    "logical-resource".len() + 2
                );
                let plan = tree_table_plan(&app, tree, &app.visible()).unwrap();
                assert_eq!(
                    plan.header().width() + 1,
                    hidden.horizontal.pinned_width
                        + hidden.horizontal.content_width
                        + hidden.horizontal.right_pinned_width
                );
                let right = hidden.horizontal.pinned_width + hidden.horizontal.viewport_width;
                let pinned = hidden.horizontal.pinned_width;
                let right_pinned = hidden.horizontal.right_pinned_width;
                for (before, hidden) in before.lines.iter().zip(&hidden.lines) {
                    assert_eq!(
                        horizontal_slice(before, 0, pinned),
                        horizontal_slice(hidden, 0, pinned)
                    );
                    assert_eq!(
                        horizontal_slice(before, right, right_pinned),
                        horizontal_slice(hidden, right, right_pinned)
                    );
                }
                app.apply_snapshot(Snapshot::parse(&serde_json::to_vec(&trace).unwrap()).unwrap());
                assert!(!app.show_logical_name);
                assert_eq!(rendered_tree(&app, tree).unwrap().lines, hidden.lines);
                app.handle_key(KeyEvent::new(KeyCode::Char('L'), KeyModifiers::NONE), area);
                assert!(app.show_logical_name);
                assert_eq!(rendered_tree(&app, tree).unwrap().lines, before.lines);
            }
        }
    }
}

#[test]
fn logical_name_toggle_clamps_horizontal_scroll_without_changing_selection() {
    let mut app = app();
    app.show_logical_name = true;
    let area = Rect::new(0, 0, 50, 16);
    let tree = resource_tree_area(area).unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE), area);
    let selected = app.selected_node().unwrap().identity.clone();
    app.resource_horizontal_scroll = usize::MAX;
    app.clamp_tree_horizontal_scroll(area);
    let before = app.resource_horizontal_scroll;
    app.handle_key(KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT), area);
    let hidden = rendered_tree(&app, tree).unwrap();
    assert_eq!(
        app.resource_horizontal_scroll,
        hidden.horizontal.max_offset()
    );
    assert!(app.resource_horizontal_scroll < before);
    assert_eq!(app.selected_node().unwrap().identity, selected);
    let offset = app.resource_horizontal_scroll;
    app.handle_key(KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT), area);
    assert_eq!(app.resource_horizontal_scroll, offset);
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE), area);
    assert!(app.show_logical_name);
    assert_eq!(app.resource_horizontal_scroll, offset + 4);
}

#[test]
fn logical_name_toggle_does_not_intercept_text_inputs_or_help() {
    let mut app = app();
    let area = Rect::new(0, 0, 100, 16);
    for (key, modifiers) in [
        ('/', KeyModifiers::NONE),
        ('f', KeyModifiers::CONTROL),
        (':', KeyModifiers::NONE),
    ] {
        app.handle_key(KeyEvent::new(KeyCode::Char(key), modifiers), area);
        app.handle_key(KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT), area);
        assert!(!app.show_logical_name);
        assert_eq!(app.input, "L");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), area);
    }
    app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE), area);
    app.handle_key(KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT), area);
    assert!(!app.show_logical_name);
    assert_eq!(app.mode, InputMode::Help);
}

#[test]
fn logical_name_annotation_is_sanitized_and_missing_or_non_string_values_use_dash() {
    for (annotation, expected) in [
        (
            serde_json::json!("first\nsecond\t\u{1b}[31m"),
            "first second \\x1b[31m",
        ),
        (serde_json::Value::Null, "-"),
        (serde_json::json!(42), "-"),
    ] {
        let mut app = app();
        app.show_logical_name = true;
        let trace = serde_json::json!({"object": {
            "apiVersion": "v1", "kind": "Root",
            "metadata": {"name": "root", "annotations": {
                "crossplane.io/composition-resource-name": annotation
            }}
        }});
        app.apply_snapshot(Snapshot::parse(&serde_json::to_vec(&trace).unwrap()).unwrap());
        let rendered =
            rendered_tree(&app, resource_tree_area(Rect::new(0, 0, 240, 16)).unwrap()).unwrap();
        assert_eq!(
            horizontal_slice(
                &rendered.lines[1],
                rendered.horizontal.pinned_width,
                expected.width()
            ),
            expected
        );
        assert!(!rendered.lines[1].contains(['\n', '\t', '\u{1b}']));
    }
}

#[test]
fn narrow_table_keeps_timestamp_columns_scrollable_unless_short_is_enabled() {
    let app = app();
    let snapshot = app.snapshot.as_ref().unwrap();
    let visible = app.visible();
    let plan = TablePlan::new(snapshot, &visible, 50, false, false, false);
    let header = plan.header();
    assert!(header.contains("SYNCED LAST"));
    assert!(header.contains("READY LAST"));
    assert!(plan.horizontal_layout(0).max_offset() > 0);
    let short = TablePlan::new(snapshot, &visible, 50, false, true, false);
    let header = short.header();
    assert!(!header.contains("LAST"));
    assert!(header.contains("SYNCED"));
    assert!(header.contains("READY"));
}

#[test]
fn external_name_column_order_and_toggle_preserve_pinned_regions_in_all_modes() {
    for (api_version, kind) in [
        ("example.io/v1", "Root"),
        ("pkg.crossplane.io/v1", "Provider"),
    ] {
        for short in [false, true] {
            for ascii in [false, true] {
                let mut app = app();
                let external = "external-界界-resource";
                let trace = serde_json::json!({"object": {
                    "apiVersion": api_version, "kind": kind,
                    "metadata": {"name": "root", "annotations": {
                        "crossplane.io/composition-resource-name": "logical-resource",
                        "crossplane.io/external-name": external
                    }}
                }, "children": [{"object": {
                    "apiVersion": "v1", "kind": "Secret", "metadata": {"name": "child"}
                }}]});
                let bytes = serde_json::to_vec(&trace).unwrap();
                app.apply_snapshot(Snapshot::parse(&bytes).unwrap());
                app.config.ui.short = short;
                app.config.ui.ascii = ascii;
                let area = Rect::new(0, 0, 240, 16);
                let tree = resource_tree_area(area).unwrap();
                let hidden = rendered_tree(&app, tree).unwrap();
                assert!(!hidden.lines[0].contains("LOGICAL NAME"));
                assert!(!hidden.lines[0].contains("EXTERNAL NAME"));
                app.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT), area);
                assert!(app.show_external_name);
                assert!(!app.show_logical_name);
                let shown = rendered_tree(&app, tree).unwrap();
                let start = shown.horizontal.pinned_width;
                assert_eq!(
                    horizontal_slice(&shown.lines[0], start, 13),
                    "EXTERNAL NAME"
                );
                assert_eq!(
                    horizontal_slice(&shown.lines[1], start, external.width()),
                    external
                );
                assert_eq!(
                    horizontal_slice(&shown.lines[2], start, external.width()).trim(),
                    "-"
                );
                assert_eq!(
                    shown.horizontal.content_width - hidden.horizontal.content_width,
                    external.width() + 2
                );
                let right = start + shown.horizontal.viewport_width;
                for (hidden, shown) in hidden.lines.iter().zip(&shown.lines) {
                    assert_eq!(
                        horizontal_slice(hidden, 0, start),
                        horizontal_slice(shown, 0, start)
                    );
                    assert_eq!(
                        horizontal_slice(hidden, right, 9),
                        horizontal_slice(shown, right, 9)
                    );
                }
                app.apply_snapshot(Snapshot::parse(&bytes).unwrap());
                assert_eq!(rendered_tree(&app, tree).unwrap().lines, shown.lines);
                app.handle_key(KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT), area);
                let plan = tree_table_plan(&app, tree, &app.visible()).unwrap();
                let header = plan.header();
                assert!(
                    header.find("LOGICAL NAME").unwrap() < header.find("EXTERNAL NAME").unwrap()
                );
                assert_eq!(
                    header.width() + 1,
                    plan.horizontal_layout(0).pinned_width
                        + plan.horizontal_layout(0).content_width
                        + plan.horizontal_layout(0).right_pinned_width
                );
                app.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE), area);
                assert!(!app.show_external_name);
                assert!(app.show_logical_name);
            }
        }
    }
}

#[test]
fn external_name_visibility_uses_config_and_reload_applies_only_changed_values() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    std::fs::write(&path, "[ui]\nshow_external_name = true\n").unwrap();
    let cli = Cli::parse_from(["xpdelve", "--config", path.to_str().unwrap(), "Root/root"]);
    let config = Config::load(&cli).unwrap();
    let theme = Theme::resolve(&config.skin, config.ui.color).unwrap();
    let mut app = App::new("Root/root".into(), config, &cli, theme);
    let area = Rect::new(0, 0, 100, 16);
    assert!(app.show_external_name);
    assert!(!app.show_logical_name);
    app.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT), area);
    assert!(!app.show_external_name);
    app.reload_config();
    assert!(!app.show_external_name);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "[ui]\nshow_external_name = true\n"
    );
    std::fs::write(&path, "[ui]\nshow_external_name = false\n").unwrap();
    app.reload_config();
    assert!(!app.show_external_name);
    std::fs::write(&path, "[ui]\nshow_external_name = true\n").unwrap();
    app.reload_config();
    assert!(app.show_external_name);
    std::fs::write(&path, "[ui]\nshort = true\n").unwrap();
    app.reload_config();
    assert!(!app.show_external_name);
}

#[test]
fn external_name_toggle_clamps_scroll_and_does_not_intercept_text_inputs_or_help() {
    let mut app = app();
    let area = Rect::new(0, 0, 50, 16);
    app.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT), area);
    app.resource_horizontal_scroll = usize::MAX;
    app.clamp_tree_horizontal_scroll(area);
    let before = app.resource_horizontal_scroll;
    let selected = app.selected_node().unwrap().identity.clone();
    app.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT), area);
    assert!(app.resource_horizontal_scroll < before);
    let rendered = rendered_tree(&app, resource_tree_area(area).unwrap()).unwrap();
    assert_eq!(
        app.resource_horizontal_scroll,
        rendered.horizontal.max_offset()
    );
    assert_eq!(app.selected_node().unwrap().identity, selected);
    for (key, modifiers) in [
        ('/', KeyModifiers::NONE),
        ('f', KeyModifiers::CONTROL),
        (':', KeyModifiers::NONE),
    ] {
        app.handle_key(KeyEvent::new(KeyCode::Char(key), modifiers), area);
        app.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT), area);
        assert!(!app.show_external_name);
        assert_eq!(app.input, "X");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), area);
    }
    app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE), area);
    app.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT), area);
    assert!(!app.show_external_name);
    assert_eq!(app.mode, InputMode::Help);
}

#[test]
fn external_name_annotation_is_sanitized_and_non_string_values_use_dash() {
    for (annotation, expected) in [
        (
            serde_json::json!("first\nsecond\t\u{1b}[31m"),
            "first second \\x1b[31m",
        ),
        (serde_json::Value::Null, "-"),
        (serde_json::json!(42), "-"),
    ] {
        let mut app = app();
        app.show_external_name = true;
        let trace = serde_json::json!({"object": {
            "apiVersion": "v1", "kind": "Secret",
            "metadata": {"name": "root", "annotations": {
                "crossplane.io/external-name": annotation
            }}
        }});
        app.apply_snapshot(Snapshot::parse(&serde_json::to_vec(&trace).unwrap()).unwrap());
        let rendered =
            rendered_tree(&app, resource_tree_area(Rect::new(0, 0, 240, 16)).unwrap()).unwrap();
        assert_eq!(
            horizontal_slice(
                &rendered.lines[1],
                rendered.horizontal.pinned_width,
                expected.width()
            ),
            expected
        );
        assert!(!rendered.lines[1].contains(['\n', '\t', '\u{1b}']));
    }
}

#[test]
fn tree_horizontal_keys_scroll_columns_but_keep_objects_pinned() {
    let mut app = app();
    let area = Rect::new(0, 0, 50, 16);
    let tree = resource_tree_area(area).unwrap();
    let before = rendered_tree(&app, tree).unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT), area);
    let after = rendered_tree(&app, tree).unwrap();
    let pinned = before.horizontal.pinned_width;
    assert_eq!(after.horizontal.offset, 4);
    assert_ne!(before.lines[0], after.lines[0]);
    assert_eq!(
        horizontal_slice(&before.lines[0], 0, pinned),
        horizontal_slice(&after.lines[0], 0, pinned)
    );
    assert_eq!(
        horizontal_slice(&before.lines[1], 0, pinned),
        horizontal_slice(&after.lines[1], 0, pinned)
    );
    assert_eq!(horizontal_slice(&after.lines[0], pinned, 1), "P");
    assert_eq!(app.selection.visible_index, 0);
    app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE), area);
    assert_eq!(rendered_tree(&app, tree).unwrap().lines, before.lines);
}

#[test]
fn object_and_status_are_pinned_while_only_middle_columns_scroll() {
    for (api_version, kind) in [
        ("example.io/v1", "Root"),
        ("pkg.crossplane.io/v1", "Provider"),
    ] {
        for short in [false, true] {
            let mut app = app_with_extra_columns();
            let trace = serde_json::json!({"object":{"apiVersion":api_version,"kind":kind,"metadata":{"name":"root"},"status":{"conditions":[{"type":"Ready","status":"False","reason":"Creating"},{"type":"Healthy","status":"False","reason":"Creating"}]}}});
            app.apply_snapshot(Snapshot::parse(&serde_json::to_vec(&trace).unwrap()).unwrap());
            app.config.ui.short = short;
            for width in [32, 50, 100, 240] {
                let tree = resource_tree_area(Rect::new(0, 0, width, 16)).unwrap();
                app.resource_horizontal_scroll = 0;
                let before = rendered_tree(&app, tree).unwrap();
                let horizontal = before.horizontal;
                let right = horizontal.pinned_width + horizontal.viewport_width;
                assert_eq!(before.lines[0].matches("STATUS").count(), 1);
                assert_eq!(
                    right + horizontal.right_pinned_width,
                    usize::from(width - 2)
                );
                assert_eq!(
                    horizontal_slice(
                        &before.lines[0],
                        right + 2,
                        horizontal.right_pinned_width - 3
                    )
                    .trim(),
                    "STATUS"
                );
                assert_eq!(
                    horizontal_slice(
                        &before.lines[1],
                        right + 2,
                        horizontal.right_pinned_width - 3
                    )
                    .trim(),
                    "Creating"
                );
                for offset in [4, usize::MAX] {
                    app.resource_horizontal_scroll = offset;
                    let after = rendered_tree(&app, tree).unwrap();
                    for (before, after) in before.lines.iter().zip(&after.lines) {
                        assert_eq!(
                            horizontal_slice(before, 0, horizontal.pinned_width),
                            horizontal_slice(after, 0, horizontal.pinned_width)
                        );
                        assert_eq!(
                            horizontal_slice(before, right, horizontal.right_pinned_width),
                            horizontal_slice(after, right, horizontal.right_pinned_width)
                        );
                        assert_eq!(after.width(), usize::from(width - 2));
                    }
                }
            }
        }
    }
}

#[test]
fn long_unicode_statuses_are_capped_without_changing_model_text() {
    let mut app = app();
    let mut snapshot = app.snapshot.as_ref().unwrap().as_ref().clone();
    let status = format!("{}tail", "界".repeat(50));
    Arc::make_mut(&mut snapshot.nodes)[0].status = status.clone();
    app.apply_snapshot(snapshot);
    let middle_width = tree_table_plan(
        &app,
        resource_tree_area(Rect::new(0, 0, 100, 16)).unwrap(),
        &app.visible(),
    )
    .unwrap()
    .horizontal_layout(0)
    .content_width;
    for width in [32, 50, 100, 240] {
        let tree = resource_tree_area(Rect::new(0, 0, width, 16)).unwrap();
        let rendered = rendered_tree(&app, tree).unwrap();
        let horizontal = rendered.horizontal;
        let status_width = horizontal.right_pinned_width - 3;
        assert!((6..=24).contains(&status_width));
        assert!(rendered.lines[1].trim_end().ends_with('…'));
        assert_eq!(rendered.lines[1].width(), usize::from(width - 2));
        assert_eq!(horizontal.content_width, middle_width);
    }
    assert_eq!(app.snapshot.as_ref().unwrap().nodes[0].status, status);
}

#[test]
fn status_has_one_reserved_space_before_the_right_border() {
    for (api_version, kind, status) in [
        ("example.io/v1", "Root", "Creating"),
        ("pkg.crossplane.io/v1", "Provider", "HealthyPackageRevision"),
        (
            "example.io/v1",
            "Root",
            "A very long successful status that needs truncation",
        ),
    ] {
        let mut app = app();
        let trace = serde_json::json!({"object":{"apiVersion":api_version,"kind":kind,"metadata":{"name":"root"}}});
        let mut snapshot = Snapshot::parse(&serde_json::to_vec(&trace).unwrap()).unwrap();
        Arc::make_mut(&mut snapshot.nodes)[0].status = status.into();
        app.apply_snapshot(snapshot);
        for width in [32, 50, 100, 240] {
            let mut terminal = Terminal::new(TestBackend::new(width, 16)).unwrap();
            terminal.draw(|frame| render(frame, &app)).unwrap();
            let buffer = terminal.backend().buffer();
            assert_eq!(buffer.cell((width - 2, 3)).unwrap().symbol(), " ");
            assert_ne!(buffer.cell((width - 3, 3)).unwrap().symbol(), " ");
            let rendered = rendered_tree(
                &app,
                resource_tree_area(Rect::new(0, 0, width, 16)).unwrap(),
            )
            .unwrap();
            assert_eq!(rendered.lines[1].width(), usize::from(width - 2));
            assert!(rendered.lines[0].ends_with(' '));
            assert!(rendered.lines[1].ends_with(' '));
        }
    }
}

#[test]
fn status_rendering_is_single_line_and_zero_middle_space_cannot_scroll() {
    let mut app = app();
    let mut snapshot = app.snapshot.as_ref().unwrap().as_ref().clone();
    Arc::make_mut(&mut snapshot.nodes)[0].status = "first\nsecond\tline".into();
    app.apply_snapshot(snapshot);
    let tree = resource_tree_area(Rect::new(0, 0, 100, 16)).unwrap();
    let rendered = rendered_tree(&app, tree).unwrap();
    assert!(rendered.lines[1].ends_with("first second line "));
    assert!(!rendered.lines[1].contains(['\n', '\t']));
    let plan = TablePlan::new(
        app.snapshot.as_ref().unwrap(),
        &app.visible(),
        4,
        false,
        false,
        false,
    );
    let horizontal = plan.horizontal_layout(usize::MAX);
    assert_eq!(horizontal.viewport_width, 0);
    assert_eq!(horizontal.max_offset(), 0);
    assert_eq!(horizontal.offset, 0);
}

#[test]
fn tree_thumbwheel_and_shift_wheel_scroll_columns() {
    let mut app = app();
    let area = Rect::new(0, 0, 50, 16);
    let tree = resource_tree_area(area).unwrap();
    let before = rendered_tree(&app, tree).unwrap().lines;
    let event = MouseEvent {
        kind: MouseEventKind::ScrollRight,
        column: 20,
        row: 3,
        modifiers: KeyModifiers::NONE,
    };
    app.handle_mouse(event, area);
    let after = rendered_tree(&app, tree).unwrap().lines;
    assert_ne!(after, before);
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::ScrollUp,
            modifiers: KeyModifiers::SHIFT,
            ..event
        },
        area,
    );
    assert_eq!(rendered_tree(&app, tree).unwrap().lines, before);
}

#[test]
fn tree_horizontal_scroll_is_always_available() {
    let mut app = app();
    let area = Rect::new(0, 0, 50, 16);
    let tree = resource_tree_area(area).unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE), area);
    assert_eq!(app.resource_horizontal_scroll, 4);
    for _ in 0..100 {
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE), area);
    }
    let rendered = rendered_tree(&app, tree).unwrap();
    assert_eq!(
        app.resource_horizontal_scroll,
        rendered.horizontal.max_offset()
    );
    assert!(rendered.lines[0].ends_with("STATUS "));
    assert!(rendered.lines[1].contains("Root/root"));
    for _ in 0..100 {
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT), area);
    }
    assert_eq!(app.resource_horizontal_scroll, 0);
    // Unmodified arrows still navigate the tree.
    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), area);
    assert_eq!(app.selection.visible_index, 1);
    app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), area);
    assert_eq!(app.selection.visible_index, 0);
}

#[test]
fn tree_horizontal_scroll_preserves_and_clamps_position_on_resize() {
    let mut app = app();
    let area = Rect::new(0, 0, 50, 16);
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE), area);
    app.clamp_tree_horizontal_scroll(Rect::new(0, 0, 60, 16));
    assert_eq!(app.resource_horizontal_scroll, 4);
    app.resource_horizontal_scroll = usize::MAX;
    app.clamp_tree_horizontal_scroll(area);
    let old_offset = app.resource_horizontal_scroll;
    app.clamp_tree_horizontal_scroll(Rect::new(0, 0, 70, 16));
    assert!(app.resource_horizontal_scroll < old_offset);
    app.clamp_tree_horizontal_scroll(Rect::new(0, 0, 20, 5));
    assert!(app.resource_horizontal_scroll > 0);
    app.clamp_tree_horizontal_scroll(Rect::new(0, 0, 140, 16));
    assert_eq!(app.resource_horizontal_scroll, 0);
}

#[test]
fn tree_horizontal_scroll_clamps_after_content_changes() {
    let mut app = app();
    let area = Rect::new(0, 0, 50, 16);
    let mut snapshot = app.snapshot.as_ref().unwrap().as_ref().clone();
    Arc::make_mut(&mut snapshot.nodes)[1].status = format!("{}TAIL", "x".repeat(100));
    app.apply_snapshot(snapshot.clone());
    app.resource_horizontal_scroll = usize::MAX;
    app.clamp_tree_horizontal_scroll(area);
    let wide_offset = app.resource_horizontal_scroll;
    let rendered = rendered_tree(&app, resource_tree_area(area).unwrap()).unwrap();
    assert!(rendered.lines[2].trim_end().ends_with('…'));
    app.apply_snapshot(snapshot);
    app.clamp_tree_horizontal_scroll(area);
    assert_eq!(app.resource_horizontal_scroll, wide_offset);
    app.collapsed
        .insert(app.snapshot.as_ref().unwrap().nodes[0].identity.clone());
    app.clamp_tree_horizontal_scroll(area);
    assert!(app.resource_horizontal_scroll < wide_offset);
    let narrow_offset = app.resource_horizontal_scroll;
    app.collapsed.clear();
    app.clamp_tree_horizontal_scroll(area);
    assert_eq!(app.resource_horizontal_scroll, narrow_offset);
    app.filter = "does-not-exist".into();
    app.clamp_tree_horizontal_scroll(area);
    let filtered_offset = app.resource_horizontal_scroll;
    assert!(filtered_offset <= narrow_offset);
    app.filter.clear();
    app.apply_snapshot(
        Snapshot::parse(
            br#"{"object":{"apiVersion":"v1","kind":"Root","metadata":{"name":"root"}}}"#,
        )
        .unwrap(),
    );
    app.clamp_tree_horizontal_scroll(area);
    assert!(app.resource_horizontal_scroll <= filtered_offset);
}

#[test]
fn tree_pins_and_caps_long_unicode_objects_for_both_schemas() {
    for (api_version, kind) in [
        ("example.io/v1", "Root"),
        ("pkg.crossplane.io/v1", "Provider"),
    ] {
        let mut app = app();
        let trace = serde_json::json!({"object": {"apiVersion": api_version, "kind": kind, "metadata": {"name": "界".repeat(80)}}});
        app.apply_snapshot(Snapshot::parse(&serde_json::to_vec(&trace).unwrap()).unwrap());
        let area = resource_tree_area(Rect::new(0, 0, 50, 16)).unwrap();
        let before = rendered_tree(&app, area).unwrap();
        assert_eq!(before.horizontal.pinned_width, 24); // Object shares space remaining after status/dividers.
        assert!(before.lines[1].contains('…'));
        assert!(before.lines.iter().all(|line| line.width() <= 48));
        app.resource_horizontal_scroll = usize::MAX;
        let after = rendered_tree(&app, area).unwrap();
        assert_eq!(
            horizontal_slice(&before.lines[1], 0, before.horizontal.pinned_width),
            horizontal_slice(&after.lines[1], 0, after.horizontal.pinned_width)
        );
        assert!(after.lines[0].trim_end().ends_with("STATUS"));
        assert!(after.lines.iter().all(|line| line.width() <= 48));
        let plan = tree_table_plan(&app, area, &app.visible()).unwrap();
        assert_eq!(
            plan.header().width() + 1
                - after.horizontal.pinned_width
                - after.horizontal.right_pinned_width,
            after.horizontal.content_width
        );
    }
}

#[test]
fn tree_vertical_wheel_moves_selection_scrolls_and_clamps() {
    let mut app = app();
    let children: Vec<_> = (0..20)
        .map(|index| {
            serde_json::json!({"object": {
                "apiVersion": "v1", "kind": "Child", "metadata": {"name": format!("child-{index}")}
            }})
        })
        .collect();
    let trace = serde_json::json!({
        "object": {"apiVersion": "v1", "kind": "Root", "metadata": {"name": "root"}},
        "children": children
    });
    app.apply_snapshot(Snapshot::parse(&serde_json::to_vec(&trace).unwrap()).unwrap());
    let area = Rect::new(0, 0, 50, 10);
    let tree_area = resource_tree_area(area).unwrap();
    let event = MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: tree_area.x + 1,
        row: tree_area.y + 2,
        modifiers: KeyModifiers::NONE,
    };
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            ..event
        },
        area,
    );
    assert!(app.tree_selection.is_some());
    app.last_tree_click = Some(TreeClick {
        identity: app.selection.identity.clone().unwrap(),
        at: Instant::now(),
    });
    assert!(app.handle_mouse(event, area).is_none());
    assert_eq!(app.selection.visible_index, 3);
    assert!(app.tree_selection.is_none());
    assert!(app.last_tree_click.is_none());
    app.handle_mouse(event, area);
    assert_eq!(app.selection.visible_index, 6);
    assert!(app.resource_scroll > 0);
    assert_eq!(
        app.selection.identity,
        Some(app.selected_node().unwrap().identity.clone())
    );
    let rendered = rendered_tree(&app, tree_area).unwrap();
    assert!(rendered.start <= app.selection.visible_index);
    assert!(app.selection.visible_index < rendered.start + rendered.row_count);
    for _ in 0..20 {
        app.handle_mouse(event, area);
    }
    assert_eq!(app.selection.visible_index, 20);
    for _ in 0..20 {
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                ..event
            },
            area,
        );
    }
    assert_eq!(app.selection.visible_index, 0);
    assert_eq!(app.resource_scroll, 0);
    assert_eq!(app.resource_horizontal_scroll, 0);
}

#[test]
fn tree_vertical_wheel_ignores_outside_pointer_and_blocked_views() {
    let mut app = app();
    let area = Rect::new(0, 0, 50, 16);
    let event = MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 20,
        row: 3,
        modifiers: KeyModifiers::NONE,
    };
    app.handle_mouse(MouseEvent { row: 0, ..event }, area);
    app.handle_mouse(
        MouseEvent {
            column: area.right(),
            ..event
        },
        area,
    );
    app.mode = InputMode::Command;
    app.handle_mouse(event, area);
    app.mode = InputMode::Normal;
    app.resource_missing = true;
    app.handle_mouse(event, area);
    app.resource_missing = false;
    app.snapshot = None;
    app.handle_mouse(event, area);
    assert_eq!(app.selection.visible_index, 0);
    assert_eq!(app.resource_scroll, 0);
}

#[test]
fn tree_horizontal_wheel_ignores_other_views_and_outside_pointer() {
    let mut app = app();
    let area = Rect::new(0, 0, 50, 16);
    let event = MouseEvent {
        kind: MouseEventKind::ScrollRight,
        column: 20,
        row: 3,
        modifiers: KeyModifiers::NONE,
    };
    app.handle_mouse(MouseEvent { row: 0, ..event }, area);
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            ..event
        },
        area,
    );
    app.mode = InputMode::Help;
    app.handle_mouse(event, area);
    app.mode = InputMode::Command;
    app.handle_mouse(event, area);
    app.mode = InputMode::Normal;
    assert_eq!(app.resource_horizontal_scroll, 0);
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            modifiers: KeyModifiers::SHIFT,
            ..event
        },
        area,
    );
    assert_eq!(app.resource_horizontal_scroll, 4);
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::ScrollLeft,
            ..event
        },
        area,
    );
    assert_eq!(app.resource_horizontal_scroll, 0);
}

#[test]
fn tree_scrolled_drag_copies_object_middle_and_status_text() {
    let mut app = app();
    app.apply_snapshot(Snapshot::parse(br#"{"object":{"apiVersion":"v1","kind":"Root","metadata":{"name":"root"}},"error":{"code":404,"message":"object missing"},"children":[{"object":{"apiVersion":"v1","kind":"Child","metadata":{"name":"child"}}}]}"#).unwrap());
    let area = Rect::new(0, 0, 50, 16);
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE), area);
    let tree_area = resource_tree_area(area).unwrap();
    let body = Block::default().borders(Borders::ALL).inner(tree_area);
    let rendered = rendered_tree(&app, tree_area).unwrap();
    assert!(rendered.lines[1].trim_end().ends_with("Deleted"));
    let mouse = |kind, column| MouseEvent {
        kind,
        column,
        row: body.y + 1,
        modifiers: KeyModifiers::NONE,
    };
    app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), body.x), area);
    app.handle_mouse(
        mouse(MouseEventKind::Drag(MouseButton::Left), body.right() - 1),
        area,
    );
    assert_eq!(
        copied_mouse_text(app.handle_mouse(
            mouse(MouseEventKind::Up(MouseButton::Left), body.right() - 1),
            area
        )),
        Some(rendered.lines[1].clone())
    );
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::ScrollRight,
            column: body.x,
            row: body.y,
            modifiers: KeyModifiers::NONE,
        },
        area,
    );
    assert!(app.tree_selection.is_none());
}

#[test]
fn tree_divider_separates_pinned_objects_without_changing_column_widths() {
    for ascii in [false, true] {
        let mut app = app();
        app.config.ui.ascii = ascii;
        let area = Rect::new(0, 0, 50, 16);
        let tree = resource_tree_area(area).unwrap();
        let before = rendered_tree(&app, tree).unwrap();
        let divider = before.horizontal.pinned_width - 2;
        for line in &before.lines {
            assert_eq!(
                horizontal_slice(line, divider, 2),
                if ascii { "| " } else { "│ " }
            );
            assert_eq!(line.width(), usize::from(tree.width - 2));
        }
        assert_eq!(horizontal_slice(&before.lines[0], divider + 2, 5), "GROUP");
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE), area);
        let after = rendered_tree(&app, tree).unwrap();
        for (before, after) in before.lines.iter().zip(&after.lines) {
            assert_eq!(
                horizontal_slice(before, 0, divider + 2),
                horizontal_slice(after, 0, divider + 2)
            );
        }
        let mut terminal = Terminal::new(TestBackend::new(50, 16)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        for row in 2..=4 {
            assert_eq!(
                terminal
                    .backend()
                    .buffer()
                    .cell((divider as u16 + 1, row))
                    .unwrap()
                    .symbol(),
                if ascii { "|" } else { "│" }
            );
        }
    }
}

#[test]
fn tree_divider_uses_subtle_style_without_changing_row_selection() {
    for color in [
        crate::config::ColorMode::Always,
        crate::config::ColorMode::Never,
    ] {
        for ascii in [false, true] {
            let mut app = app_with_theme("catppuccin-mocha", color);
            app.config.ui.ascii = ascii;
            let mut snapshot = app.snapshot.as_ref().unwrap().as_ref().clone();
            Arc::make_mut(&mut snapshot.nodes)[1].health = crate::model::Health::Unhealthy;
            app.apply_snapshot(snapshot);
            let tree = resource_tree_area(Rect::new(0, 0, 50, 16)).unwrap();
            let layout = rendered_tree(&app, tree).unwrap().horizontal;
            let divider = tree.x + 1 + layout.pinned_width as u16 - 2;
            let status_divider = tree.x + 1 + (layout.pinned_width + layout.viewport_width) as u16;
            let mut terminal = Terminal::new(TestBackend::new(50, 16)).unwrap();
            terminal.draw(|frame| render(frame, &app)).unwrap();
            let buffer = terminal.backend().buffer();
            for row in 2..=4 {
                let cell = buffer.cell((divider, row)).unwrap();
                let status_cell = buffer.cell((status_divider, row)).unwrap();
                assert_eq!(cell.symbol(), if ascii { "|" } else { "│" });
                assert_eq!(status_cell.symbol(), cell.symbol());
                assert_eq!(status_cell.fg, cell.fg);
                assert_eq!(status_cell.bg, cell.bg);
                assert_eq!(status_cell.modifier, cell.modifier);
                if app.theme.colors_enabled {
                    assert_eq!(cell.fg, app.theme.palette.overlay1);
                } else {
                    assert_eq!(cell.fg, Color::Reset);
                    assert!(cell.modifier.contains(Modifier::DIM));
                }
            }
            let selected = buffer.cell((divider, 3)).unwrap();
            assert_eq!(selected.bg, buffer.cell((divider - 1, 3)).unwrap().bg);
            assert_eq!(
                selected.modifier.contains(Modifier::REVERSED),
                !app.theme.colors_enabled
            );
            assert_eq!(
                buffer.cell((divider - 1, 4)).unwrap().fg,
                app.theme
                    .health(crate::model::Health::Unhealthy)
                    .fg
                    .unwrap_or(Color::Reset)
            );
            let rendered = rendered_tree(&app, tree).unwrap();
            let symbol = if ascii { "|" } else { "│" };
            let start = rendered.lines[0].len() + 1 + rendered.lines[1].find(symbol).unwrap();
            let point = SelectionPoint {
                start,
                end: start + symbol.len(),
            };
            app.tree_selection = Some(TreeSelection {
                text: TextSelection {
                    anchor: point,
                    focus: point,
                    dragged: true,
                },
                content: rendered.content,
                area: tree,
                selecting: false,
            });
            terminal.draw(|frame| render(frame, &app)).unwrap();
            let highlighted = terminal.backend().buffer().cell((divider, 3)).unwrap();
            if app.theme.colors_enabled {
                assert_eq!(highlighted.bg, app.theme.palette.yellow);
                assert_eq!(highlighted.fg, app.theme.palette.base);
            } else {
                assert!(!highlighted.modifier.contains(Modifier::REVERSED));
                assert!(highlighted.modifier.contains(Modifier::UNDERLINED));
            }
        }
    }
}

#[test]
fn tree_scrollbar_has_static_square_thumb_and_left_arrow_in_separator_gap() {
    for color in [
        crate::config::ColorMode::Always,
        crate::config::ColorMode::Never,
    ] {
        for ascii in [false, true] {
            let mut app = app_with_theme("catppuccin-mocha", color);
            app.config.ui.ascii = ascii;
            let tree = resource_tree_area(Rect::new(0, 0, 50, 16)).unwrap();
            let layout = rendered_tree(&app, tree).unwrap().horizontal;
            let mut terminal = Terminal::new(TestBackend::new(50, 16)).unwrap();
            let left = tree.x + layout.pinned_width as u16;
            let right = left + layout.viewport_width as u16;
            let bottom = tree.bottom() - 1;
            for offset in [0, 4, layout.max_offset()] {
                app.resource_horizontal_scroll = offset;
                terminal.draw(|frame| render(frame, &app)).unwrap();
                let buffer = terminal.backend().buffer();
                let start = buffer.cell((left, bottom)).unwrap();
                let end = buffer.cell((right, bottom)).unwrap();
                assert_eq!(start.symbol(), if ascii { "<" } else { "◀" });
                assert_eq!(end.symbol(), if ascii { ">" } else { "▶" });
                assert_eq!(start.modifier.contains(Modifier::DIM), offset == 0);
                assert_eq!(
                    end.modifier.contains(Modifier::DIM),
                    offset == layout.max_offset()
                );
                let thumb = (left + 1..right)
                    .map(|x| buffer.cell((x, bottom)).unwrap())
                    .find(|cell| cell.symbol() == if ascii { "-" } else { "▪" })
                    .unwrap();
                let track = (left + 1..right)
                    .map(|x| buffer.cell((x, bottom)).unwrap())
                    .find(|cell| cell.symbol() == if ascii { "." } else { "·" })
                    .unwrap();
                assert!(!thumb.modifier.contains(Modifier::BOLD));
                if app.theme.colors_enabled {
                    assert_eq!(thumb.fg, app.theme.palette.subtext1);
                    assert_eq!(track.fg, app.theme.palette.overlay1);
                    assert_ne!(thumb.fg, track.fg);
                } else {
                    assert!(track.modifier.contains(Modifier::DIM));
                }
                let unchanged = buffer.clone();
                terminal.draw(|frame| render(frame, &app)).unwrap();
                assert_eq!(terminal.backend().buffer(), &unchanged);
            }
            let mut wide_terminal = Terminal::new(TestBackend::new(140, 16)).unwrap();
            wide_terminal.draw(|frame| render(frame, &app)).unwrap();
            let wide = wide_terminal.backend().to_string();
            assert!(!wide.contains(['▪', '◀', '▶']));
            assert!(!wide.contains("<"));
        }
    }
}

#[test]
fn tree_horizontal_layout_matches_formatted_columns_across_schemas() {
    for (api_version, kind) in [
        ("example.io/v1", "Root"),
        ("pkg.crossplane.io/v1", "Provider"),
    ] {
        let mut app = app();
        let trace = serde_json::json!({"object": {"apiVersion": api_version, "kind": kind, "metadata": {"name": "a-long-resource-name"}}});
        app.apply_snapshot(Snapshot::parse(&serde_json::to_vec(&trace).unwrap()).unwrap());
        for short in [false, true] {
            app.config.ui.short = short;
            for width in 32..160 {
                let tree = resource_tree_area(Rect::new(0, 0, width, 16)).unwrap();
                let plan = tree_table_plan(&app, tree, &app.visible()).unwrap();
                let layout = plan.horizontal_layout(usize::MAX);
                assert!(layout.pinned_width - 2 <= usize::from(width - 2) * 3 / 5);
                let header = plan.header();
                assert_eq!(
                    header.width() + 1,
                    layout.pinned_width + layout.content_width + layout.right_pinned_width
                );
                for label in ["OBJECT", "STATUS"] {
                    assert!(header.contains(label));
                }
                for label in ["SYNCED LAST", "READY LAST"] {
                    assert_eq!(header.contains(label), !short && kind == "Root");
                }
                app.resource_horizontal_scroll = usize::MAX;
                let rendered = rendered_tree(&app, tree).unwrap();
                assert!(
                    rendered
                        .lines
                        .iter()
                        .all(|line| line.width() <= usize::from(width - 2))
                );
            }
        }
    }
}

#[test]
fn horizontal_slicing_preserves_graphemes_and_wide_cell_alignment() {
    assert_eq!(pad_or_truncate("👩‍💻👩‍💻👩‍💻", 5), "👩‍💻👩‍💻…");
    assert_eq!(horizontal_slice("A界BC", 2, 3), " BC");
    assert_eq!(horizontal_slice("A界BC", 0, 2), "A ");
    assert_eq!(horizontal_slice("e\u{301}界x", 0, 3), "e\u{301}界");
    assert_eq!(horizontal_slice("e\u{301}界x", 1, 3), "界x");
    assert_eq!(horizontal_slice("👩‍💻x", 1, 2), " x");
    assert_eq!(horizontal_slice("界", 1, 0), "");
    assert_eq!(horizontal_slice("text", 100, 5), "");
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
fn selected_row_style_follows_theme_and_color_mode() {
    for (name, color) in [
        ("catppuccin-mocha", crate::config::ColorMode::Always),
        ("catppuccin-latte", crate::config::ColorMode::Always),
        ("catppuccin-mocha", crate::config::ColorMode::Never),
    ] {
        let app = app_with_theme(name, color);
        let mut terminal = Terminal::new(TestBackend::new(100, 16)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        assert!(!terminal.backend().to_string().contains(">-  Root/root"));
        let cell = terminal.backend().buffer().cell((1, 3)).unwrap();
        if app.theme.colors_enabled {
            assert_eq!(cell.fg, app.theme.palette.base);
            assert_eq!(cell.bg, app.theme.palette.lavender);
        } else {
            assert_eq!(cell.fg, Color::Reset);
            assert_eq!(cell.bg, Color::Reset);
            assert!(cell.modifier.contains(Modifier::REVERSED));
            assert!(cell.modifier.contains(Modifier::BOLD));
        }
    }
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
        targets: vec![Target {
            identity: Identity {
                group: "very-long.example.crossplane.io".into(),
                version: "v1".into(),
                kind: "VeryLongResourceKind".into(),
                namespace: Some("default".into()),
                name: "a-resource-name-that-is-long-enough-to-require-truncation".into(),
            },
            expected_uid: Some("must-not-be-rendered".into()),
        }],
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
        targets: vec![FinalizerTarget {
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
        }],
        names: vec![
            ("demo.xpdelve.io/hold-for-recording".into(), 1),
            ("kubernetes.io/foregroundDeletion".into(), 1),
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
    assert!(rendered.contains("Enter:Review"), "{rendered}");
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
fn main_footer_groups_logical_name_toggle_with_tree_controls() {
    let mut app = app();
    let mut terminal = Terminal::new(TestBackend::new(240, 16)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    assert_eq!(
        terminal.backend().buffer().cell((1, 14)).unwrap().fg,
        app.theme.palette.overlay1
    );
    for visible in [true, false] {
        app.show_logical_name = visible;
        let mut terminal = Terminal::new(TestBackend::new(240, 16)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let footer = (0..240)
            .map(|column| {
                terminal
                    .backend()
                    .buffer()
                    .cell((column, 14))
                    .unwrap()
                    .symbol()
            })
            .collect::<String>();
        let navigation = footer.find("Left/Right:collapse/expand").unwrap();
        let logical_name = footer.find("L:logical-name").unwrap();
        let external_name = footer.find("X:external-name").unwrap();
        let command = footer.find("::command").unwrap();
        let actions = footer.find("ctrl-d:delete").unwrap();
        assert!(navigation < logical_name && logical_name < external_name);
        assert!(external_name < command && command < actions);
    }
}

#[test]
fn tree_scroll_hint_is_right_aligned_and_only_shown_for_overflow() {
    let mut app = app();
    let mut saw_overflow = false;
    let mut saw_no_overflow = false;
    for width in [32, 60, 100, 240, 32] {
        let mut terminal = Terminal::new(TestBackend::new(width, 20)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let footer = (0..width)
            .map(|column| {
                terminal
                    .backend()
                    .buffer()
                    .cell((column, 18))
                    .unwrap()
                    .symbol()
            })
            .collect::<String>();
        let tree = resource_tree_area(Rect::new(0, 0, width, 20)).unwrap();
        let horizontal = tree_table_plan(&app, tree, &app.visible())
            .unwrap()
            .horizontal_layout(0);
        if horizontal.max_offset() > 0 {
            saw_overflow = true;
            assert!(footer.ends_with(" h/l:scroll "), "{footer:?}");
            assert!(!footer.contains("Enter:details"));
        } else {
            saw_no_overflow = true;
            assert!(!footer.contains("h/l:scroll"));
            if width == 240 {
                assert!(footer.contains("E:events"));
            }
        }
    }
    assert!(saw_overflow && saw_no_overflow);
    app.filter = "child".into();
    let mut terminal = Terminal::new(TestBackend::new(32, 20)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let output = terminal.backend().to_string();
    assert!(output.contains("Filter: child"));
    assert!(output.contains("h/l:scroll"));

    for mode in [InputMode::Filter, InputMode::Find, InputMode::Command] {
        app.mode = mode;
        terminal.draw(|frame| render(frame, &app)).unwrap();
        assert!(!terminal.backend().to_string().contains("h/l:scroll"));
    }
    app.mode = InputMode::Normal;
    app.snapshot = None;
    terminal.draw(|frame| render(frame, &app)).unwrap();
    assert!(!terminal.backend().to_string().contains("h/l:scroll"));
}

#[test]
fn enter_hint_tracks_selected_details_not_health() {
    let mut app = app();
    app.apply_snapshot(Snapshot::parse(br#"{"object":{"apiVersion":"v1","kind":"Root","metadata":{"name":"root"},"status":{"conditions":[{"type":"Ready","status":"True"}]}},"children":[{"object":{"apiVersion":"v1","kind":"Child","metadata":{"name":"deleted"}},"error":{"code":404,"message":"object missing"}},{"object":{"apiVersion":"v1","kind":"Child","metadata":{"name":"unready"},"status":{"conditions":[{"type":"Ready","status":"False"}]}}}]}"#).unwrap());
    let mut terminal = Terminal::new(TestBackend::new(160, 20)).unwrap();
    for (position, expected) in [(0, false), (1, true), (2, true), (0, false)] {
        app.set_selection(position);
        terminal.draw(|frame| render(frame, &app)).unwrap();
        assert_eq!(
            terminal.backend().to_string().contains("Enter:details"),
            expected
        );
    }
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
    assert!(rendered.contains("Space"));
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
    assert!(rendered.contains("select finalizers to remove"));
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
        state: TextModalState::new(ContentKind::Yaml),
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
        state: TextModalState::new(ContentKind::Yaml),
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
        state: TextModalState::new(ContentKind::Error),
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
        state: TextModalState::new(ContentKind::Yaml),
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

    let Some(Modal::Text { state, .. }) = app.modal else {
        panic!("expected text modal");
    };
    assert_eq!(state.horizontal_scroll, 8);
}

#[test]
fn help_mouse_wheel_scrolls_and_clamps_without_a_snapshot() {
    let mut app = app();
    app.snapshot = None;
    app.mode = InputMode::Help;
    assert!(app.captures_mouse());
    let area = Rect::new(0, 0, 100, 24);
    let help_area = centered(area, 72, 20);
    let event = MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: help_area.x + 1,
        row: help_area.y + 1,
        modifiers: KeyModifiers::NONE,
    };
    app.handle_mouse(
        MouseEvent {
            column: 0,
            row: 0,
            ..event
        },
        area,
    );
    assert_eq!(app.help_scroll, 0);
    app.handle_mouse(event, area);
    assert_eq!(app.help_scroll, 3);
    app.handle_mouse(
        MouseEvent {
            modifiers: KeyModifiers::SHIFT,
            ..event
        },
        area,
    );
    assert_eq!(app.help_scroll, 3);
    for _ in 0..30 {
        app.handle_mouse(event, area);
    }
    assert_eq!(
        app.help_scroll,
        (HELP_LINES.len() as u16) - (help_area.height - 3)
    );
    for _ in 0..30 {
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                ..event
            },
            area,
        );
    }
    assert_eq!(app.help_scroll, 0);
    assert_eq!(app.mode, InputMode::Help);
}

#[test]
fn content_mouse_wheel_scrolls_visual_rows_and_clamps() {
    let area = Rect::new(0, 0, 40, 10);
    for kind in [
        ContentKind::Yaml,
        ContentKind::Describe,
        ContentKind::Events,
        ContentKind::Error,
        ContentKind::SmallError,
    ] {
        for wrapped in [false, true] {
            let mut app = app();
            let content = (0..20)
                .map(|index| format!("line {index}: {}", "word ".repeat(20)))
                .collect::<Vec<_>>()
                .join("\n");
            let body = content_modal_body(area, kind);
            let max_scroll = visual_rows(&content, body.width, wrapped)
                .len()
                .saturating_sub(usize::from(body.height)) as u16;
            app.modal = Some(Modal::Text {
                title: "content".into(),
                content: content.into(),
                kind,
                state: {
                    let mut state = TextModalState::new(kind);
                    state.wrapped = wrapped;
                    state
                },
                selection: None,
            });
            let event = MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: body.x,
                row: body.y,
                modifiers: KeyModifiers::NONE,
            };
            app.handle_mouse(
                MouseEvent {
                    row: area.bottom(),
                    ..event
                },
                area,
            );
            assert!(matches!(
                &app.modal,
                Some(Modal::Text { state, .. }) if state.vertical_scroll == 0
            ));
            app.handle_mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    ..event
                },
                area,
            );
            assert!(matches!(
                app.modal,
                Some(Modal::Text {
                    selection: Some(_),
                    ..
                })
            ));
            assert!(app.handle_mouse(event, area).is_none());
            assert!(matches!(
                &app.modal,
                Some(Modal::Text { state, selection: None, .. }) if state.vertical_scroll == 3
            ));
            app.handle_mouse(
                MouseEvent {
                    modifiers: KeyModifiers::SHIFT,
                    ..event
                },
                area,
            );
            assert!(matches!(
                &app.modal,
                Some(Modal::Text { state, .. }) if state.vertical_scroll == 3
            ));
            for _ in 0..100 {
                app.handle_mouse(event, area);
            }
            assert!(
                matches!(&app.modal, Some(Modal::Text { state, .. }) if state.vertical_scroll == max_scroll)
            );
            for _ in 0..100 {
                app.handle_mouse(
                    MouseEvent {
                        kind: MouseEventKind::ScrollUp,
                        ..event
                    },
                    area,
                );
            }
            assert!(matches!(
                &app.modal,
                Some(Modal::Text { state, .. }) if state.vertical_scroll == 0 && state.horizontal_scroll == 0
            ));
            assert_eq!(app.selection.visible_index, 0);
        }
    }
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
            state: TextModalState::new(content_kind),
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
fn content_drag_auto_scrolls_both_directions_and_copies_offscreen_text() {
    let area = Rect::new(0, 0, 40, 10);
    for kind in [
        ContentKind::Yaml,
        ContentKind::Describe,
        ContentKind::Events,
        ContentKind::Error,
        ContentKind::SmallError,
    ] {
        for wrapped in [false, true] {
            for upwards in [false, true] {
                let mut app = app();
                let source = (0..30)
                    .map(|index| format!("{index:02}: 界e\u{301} {}", "word ".repeat(12)))
                    .collect::<Vec<_>>()
                    .join("\n");
                let body = content_modal_body(area, kind);
                let content = content::TextContent::from(source.clone());
                let max_scroll = content.scroll_bounds(body.width, body.height, wrapped).0;
                let mut state = TextModalState::new(kind);
                state.wrapped = wrapped;
                state.vertical_scroll = if upwards { max_scroll } else { 0 };
                app.modal = Some(Modal::Text {
                    title: "Content".into(),
                    content,
                    kind,
                    state,
                    selection: None,
                });
                let mouse = |kind, row| MouseEvent {
                    kind,
                    column: body.x,
                    row,
                    modifiers: KeyModifiers::NONE,
                };
                let start_row = body.y + body.height / 2;
                // Drag beyond the body, then hold still: no further mouse events.
                let edge = if upwards { body.y - 1 } else { body.bottom() };
                app.handle_mouse(
                    mouse(MouseEventKind::Down(MouseButton::Left), start_row),
                    area,
                );
                app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), edge), area);
                let Some(Modal::Text {
                    selection: Some(initial),
                    ..
                }) = &app.modal
                else {
                    panic!("expected selection");
                };
                let anchor = initial.anchor;
                let first_tick = app.text_drag.unwrap().next_scroll;
                assert!(!app.scroll_text_drag(area, first_tick - Duration::from_millis(1)));
                for step in 0..max_scroll {
                    assert!(app.scroll_text_drag(
                        area,
                        first_tick + SELECTION_SCROLL_INTERVAL * u32::from(step)
                    ));
                }
                assert!(!app.scroll_text_drag(
                    area,
                    first_tick + SELECTION_SCROLL_INTERVAL * u32::from(max_scroll)
                ));
                let Some(Modal::Text {
                    state,
                    selection: Some(active),
                    ..
                }) = &app.modal
                else {
                    panic!("expected selection");
                };
                assert_eq!(state.vertical_scroll, if upwards { 0 } else { max_scroll });
                assert_eq!(active.anchor, anchor);
                let expected_focus = selection_point_at(
                    &source,
                    body,
                    body.x,
                    edge,
                    state.vertical_scroll,
                    0,
                    wrapped,
                    true,
                )
                .unwrap();
                assert_eq!(active.focus, expected_focus);
                let expected = source[active.range()].to_owned();
                assert!(expected.lines().count() > usize::from(body.height));
                assert_eq!(
                    copied_mouse_text(
                        app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), edge), area)
                    ),
                    Some(expected)
                );
                assert!(app.text_drag.is_none());
                assert!(!app.scroll_text_drag(area, first_tick + Duration::from_secs(60)));
            }
        }
    }
}

#[test]
fn content_drag_stops_scrolling_inside_body_and_on_key_or_document_change() {
    let mut app = app();
    let area = Rect::new(0, 0, 40, 10);
    let kind = ContentKind::Yaml;
    let source = "line\n".repeat(30);
    app.modal = Some(Modal::Text {
        title: "Content".into(),
        content: source.clone().into(),
        kind,
        state: TextModalState::new(kind),
        selection: None,
    });
    let body = content_modal_body(area, kind);
    let mouse = |kind, row| MouseEvent {
        kind,
        column: body.x,
        row,
        modifiers: KeyModifiers::NONE,
    };
    app.handle_mouse(
        mouse(MouseEventKind::Down(MouseButton::Left), body.y + 1),
        area,
    );
    app.handle_mouse(
        mouse(MouseEventKind::Drag(MouseButton::Left), body.bottom()),
        area,
    );
    let tick = app.text_drag.unwrap().next_scroll;
    assert!(app.scroll_text_drag(area, tick));
    app.handle_mouse(
        mouse(MouseEventKind::Drag(MouseButton::Left), body.y + 2),
        area,
    );
    assert!(!app.scroll_text_drag(area, tick + Duration::from_secs(1)));
    app.handle_mouse(
        mouse(MouseEventKind::Drag(MouseButton::Left), body.bottom()),
        area,
    );
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), area);
    assert!(app.text_drag.is_none());
    app.handle_mouse(
        mouse(MouseEventKind::Drag(MouseButton::Left), body.bottom()),
        area,
    );
    if let Some(Modal::Text { content, .. }) = &mut app.modal {
        *content = source.into();
    }
    assert!(!app.scroll_text_drag(area, tick + Duration::from_secs(2)));
    assert!(app.text_drag.is_none());
}

#[test]
fn content_wheel_preserves_drag_anchor_and_extends_copied_text() {
    let mut app = app();
    let area = Rect::new(0, 0, 40, 10);
    let kind = ContentKind::Yaml;
    let source = (0..30)
        .map(|index| format!("line {index}\n"))
        .collect::<String>();
    app.modal = Some(Modal::Text {
        title: "Content".into(),
        content: source.clone().into(),
        kind,
        state: TextModalState::new(kind),
        selection: None,
    });
    let body = content_modal_body(area, kind);
    let mouse = |kind, row| MouseEvent {
        kind,
        column: body.x,
        row,
        modifiers: KeyModifiers::NONE,
    };
    app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), body.y), area);
    app.handle_mouse(
        mouse(MouseEventKind::Drag(MouseButton::Left), body.y + 2),
        area,
    );
    app.handle_mouse_events(mouse(MouseEventKind::ScrollDown, body.y + 2), 3, area);
    let Some(Modal::Text {
        state,
        selection: Some(active),
        ..
    }) = &app.modal
    else {
        panic!("expected selection");
    };
    assert_eq!(state.vertical_scroll, 9);
    assert_eq!(active.anchor.start, 0);
    let expected = format!(
        "{}l",
        (0..11)
            .map(|index| format!("line {index}\n"))
            .collect::<String>()
    );
    assert_eq!(
        copied_mouse_text(app.handle_mouse(
            mouse(MouseEventKind::Up(MouseButton::Left), body.y + 2),
            area
        )),
        Some(expected)
    );
    app.handle_mouse(mouse(MouseEventKind::ScrollDown, body.y + 2), area);
    assert!(matches!(
        &app.modal,
        Some(Modal::Text {
            selection: None,
            ..
        })
    ));
}

#[test]
fn content_mouse_click_does_not_copy_text() {
    let mut app = app();
    app.modal = Some(Modal::Text {
        title: "YAML".into(),
        content: "kind: Widget".into(),
        kind: ContentKind::Yaml,
        state: TextModalState::new(ContentKind::Yaml),
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

    assert_eq!(app.selection.visible_index, 0);
    assert!(
        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left)), area)
            .is_none()
    );
    assert!(
        app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left)), area)
            .is_none()
    );
    assert_eq!(app.selection.visible_index, 1);
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
    assert_eq!(app.selection.visible_index, 1);
}

#[test]
fn context_menu_options_dispatch_matching_actions() {
    let area = Rect::new(0, 0, 80, 14);
    for (item, paused, expected) in [
        (0, false, "yaml"),
        (2, false, "events"),
        (3, false, "describe"),
        (4, false, "edit"),
        (5, false, "pause"),
        (5, true, "unpause"),
        (6, false, "reconcile"),
    ] {
        let mut app = app();
        app.modal = Some(Modal::ContextMenu {
            target: app.selected_target().unwrap(),
            paused,
            items: context_menu_items(paused),
            column: 10,
            row: 3,
            pressed: None,
        });
        let action = click_context_item(&mut app, area, item);
        let (actual, target) = match action {
            Some(MouseAction::Action(UiAction::Yaml(target))) => ("yaml", target),
            Some(MouseAction::Action(UiAction::Edit(target))) => ("edit", target),
            Some(MouseAction::Action(UiAction::Events(target))) => ("events", target),
            Some(MouseAction::Action(UiAction::Describe(target))) => ("describe", target),
            Some(MouseAction::Action(UiAction::SetPaused(mut targets, pause))) => {
                assert_eq!(pause, !paused);
                assert_eq!(targets.len(), 1);
                (if pause { "pause" } else { "unpause" }, targets.remove(0))
            }
            Some(MouseAction::Action(UiAction::Reconcile(mut targets))) => {
                assert_eq!(targets.len(), 1);
                ("reconcile", targets.remove(0))
            }
            _ => panic!("expected context menu action"),
        };
        assert_eq!(actual, expected);
        assert_eq!(target.identity.kind, "Root");
        assert!(app.modal.is_none());
    }
}

fn click_context_item(app: &mut App, area: Rect, item: usize) -> Option<MouseAction> {
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
    app.handle_mouse(click(MouseEventKind::Up(MouseButton::Left)), area)
}

#[test]
fn context_menu_lists_inspection_before_actions_with_a_state_aware_pause_label() {
    for (paused, label) in [(false, "Pause"), (true, "Unpause")] {
        let mut app = app();
        app.modal = Some(Modal::ContextMenu {
            target: app.selected_target().unwrap(),
            paused,
            items: context_menu_items(paused),
            column: 10,
            row: 3,
            pressed: None,
        });
        let mut terminal = Terminal::new(TestBackend::new(80, 16)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let rendered = terminal.backend().to_string();
        let position = |text: &str| rendered.find(text).unwrap_or_else(|| panic!("{text}"));
        assert!(position("YAML") < position("Status"));
        assert!(position("Status") < position("Events"));
        assert!(position("Events") < position("Describe"));
        assert!(position("Describe") < position("Edit"));
        assert!(position("Edit") < position(label));
        assert!(position(label) < position("Reconcile"));
    }
}

#[test]
fn context_menu_mutations_respect_read_only() {
    let area = Rect::new(0, 0, 80, 14);
    for (item, message) in [
        (4, "Edit is disabled"),
        (5, "Pause is disabled"),
        (6, "Reconcile is disabled"),
    ] {
        let mut app = app();
        app.config.read_only = true;
        app.modal = Some(Modal::ContextMenu {
            target: app.selected_target().unwrap(),
            paused: false,
            items: context_menu_items(false),
            column: 10,
            row: 3,
            pressed: None,
        });
        assert!(click_context_item(&mut app, area, item).is_none());
        assert!(app.status.contains(message), "{}", app.status);
        // Inspection items stay available.
        app.modal = Some(Modal::ContextMenu {
            target: app.selected_target().unwrap(),
            paused: false,
            items: context_menu_items(false),
            column: 10,
            row: 3,
            pressed: None,
        });
        assert!(matches!(
            click_context_item(&mut app, area, 0),
            Some(MouseAction::Action(UiAction::Yaml(_)))
        ));
    }
}

#[test]
fn context_menu_status_opens_the_status_modal() {
    let mut app = app();
    let area = Rect::new(0, 0, 80, 12);
    app.modal = Some(Modal::ContextMenu {
        target: app.selected_target().unwrap(),
        paused: false,
        items: context_menu_items(false),
        column: 10,
        row: 3,
        pressed: None,
    });
    let menu = context_menu_area(area, 10, 3);
    let inner = Block::default().borders(Borders::ALL).inner(menu);
    let click = |kind| MouseEvent {
        kind,
        column: inner.x,
        row: inner.y + 1,
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
        paused: false,
        items: context_menu_items(false),
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
    let plan = TablePlan::new(&snapshot, &[0], 100, true, false, false);
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
    let plan = TablePlan::new(&snapshot, &[0], 30, true, false, false);
    let header = plan.header();
    assert!(header.width() > 30);
    assert!(plan.horizontal_layout(0).max_offset() > 0);
    assert!(header.contains("INSTALLED"));
    assert!(header.contains("HEALTHY"));
    assert!(header.contains("INSTALLED LAST"));
    assert!(header.contains("HEALTHY LAST"));
    assert!(header.contains("VERSION"));
    assert!(header.contains("STATE"));
}

#[test]
fn yaml_scroll_flood_escape_and_following_keys_do_not_scroll_the_tree() {
    use super::events::{InputView, WheelInput};

    let mut app = app();
    app.finish_action(
        "YAML: Root/root",
        None,
        Ok(Some("key: value\n".repeat(2_000))),
    );
    let area = Rect::new(0, 0, 50, 16);
    let mouse = MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 20,
        row: 3,
        modifiers: KeyModifiers::NONE,
    };
    let now = Instant::now();
    let mut input = WheelInput::default();
    for _ in 0..50_000 {
        let (batch, event) = input.push(TerminalEvent::Mouse(mouse), now);
        assert!(batch.is_none());
        assert!(event.is_none());
    }
    let before = InputView::of(&app);
    let (batch, event) = input.push(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        now,
    );
    let batch = batch.unwrap();
    app.handle_mouse_events(batch.mouse, batch.count, area);
    let Some(TerminalEvent::Key(key)) = event else {
        panic!("lost Escape")
    };
    app.handle_key(key, area);
    assert!(app.modal.is_none());
    assert_ne!(before, InputView::of(&app));
    input.view_changed(now);
    for _ in 0..10_000 {
        let (batch, event) = input.push(TerminalEvent::Mouse(mouse), now);
        assert!(batch.is_none());
        assert!(event.is_none());
    }
    assert_eq!(app.selection.visible_index, 0);
    assert_eq!(app.resource_scroll, 0);
    for code in [KeyCode::Down, KeyCode::Char('q')] {
        let (batch, event) = input.push(
            TerminalEvent::Key(KeyEvent::new(code, KeyModifiers::NONE)),
            now,
        );
        assert!(batch.is_none());
        let Some(TerminalEvent::Key(key)) = event else {
            panic!("lost subsequent key")
        };
        app.handle_key(key, area);
    }
    assert_eq!(app.selection.visible_index, 1);
    assert!(app.quit);
}

#[test]
fn coalesced_wheels_match_individual_wheels_in_tree_help_and_yaml() {
    let area = Rect::new(0, 0, 50, 16);
    for view in 0..3 {
        let mut individual = app();
        let mut batched = app();
        for app in [&mut individual, &mut batched] {
            if view == 1 {
                app.mode = InputMode::Help;
            } else if view == 2 {
                app.finish_action(
                    "YAML: Root/root",
                    None,
                    Ok(Some("key: value\n".repeat(200))),
                );
            }
        }
        for (kind, modifiers) in [
            (MouseEventKind::ScrollDown, KeyModifiers::NONE),
            (MouseEventKind::ScrollUp, KeyModifiers::NONE),
            (MouseEventKind::ScrollDown, KeyModifiers::SHIFT),
            (MouseEventKind::ScrollRight, KeyModifiers::NONE),
            (MouseEventKind::ScrollLeft, KeyModifiers::NONE),
        ] {
            let mouse = MouseEvent {
                kind,
                modifiers,
                column: 20,
                row: 3,
            };
            for _ in 0..30 {
                individual.handle_mouse(mouse, area);
            }
            batched.handle_mouse_events(mouse, 30, area);
            assert_eq!(
                individual.selection.visible_index,
                batched.selection.visible_index
            );
            assert_eq!(individual.resource_scroll, batched.resource_scroll);
            assert_eq!(
                individual.resource_horizontal_scroll,
                batched.resource_horizontal_scroll
            );
            assert_eq!(individual.help_scroll, batched.help_scroll);
            if let (Some(Modal::Text { state: a, .. }), Some(Modal::Text { state: b, .. })) =
                (&individual.modal, &batched.modal)
            {
                assert_eq!(a.vertical_scroll, b.vertical_scroll);
            }
        }
    }
}

#[test]
fn replacing_a_text_document_is_an_input_view_boundary() {
    let mut app = app();
    app.finish_action("YAML: Root/root", None, Ok(Some("key: old".into())));
    let old_view = events::InputView::of(&app);
    app.finish_action("YAML: Root/root", None, Ok(Some("key: new".into())));
    assert_ne!(old_view, events::InputView::of(&app));
}

#[test]
fn modal_scroll_is_clamped_to_last_viewport() {
    let content = content::TextContent::from("one\ntwo\nthree\nfour");
    assert_eq!(content.scroll_bounds(80, 2, false).0, 2);
    let content = content::TextContent::from("0123456789");
    assert_eq!(content.scroll_bounds(5, 1, true).0, 1);
    assert_eq!(content.scroll_bounds(6, 1, false).1, 4);
}

#[test]
fn unicode_modal_matches_are_highlighted() {
    let theme = app().theme;
    let line = highlighted_line("CAFÉ", "café", Style::default(), &theme);
    assert_eq!(line.spans.len(), 1);
    assert_eq!(line.spans[0].content, "CAFÉ");
    assert_eq!(line.spans[0].style, theme.search(Style::default()));
    assert_ne!(line.spans[0].style, Style::default());
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

fn key(app: &mut App, code: KeyCode) -> UiAction {
    app.handle_key(
        KeyEvent::new(code, KeyModifiers::NONE),
        Rect::new(0, 0, 100, 20),
    )
}

#[test]
fn space_marks_without_moving_and_esc_clears_marks_before_filters() {
    let mut app = app();
    app.filter = "root".into();
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(app.marked.len(), 1);
    assert_eq!(app.selection.visible_index, 0);
    key(&mut app, KeyCode::Char(' '));
    assert!(app.marked.is_empty());
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Esc);
    assert!(app.marked.is_empty());
    assert_eq!(app.filter, "root");
    key(&mut app, KeyCode::Esc);
    assert!(app.filter.is_empty());
}

#[test]
fn checkboxes_appear_for_all_rows_once_anything_is_marked_and_stay_pinned() {
    let mut app = app();
    let area = Rect::new(0, 0, 50, 16);
    let tree = resource_tree_area(area).unwrap();
    let before = rendered_tree(&app, tree).unwrap();
    assert!(before.lines[0].starts_with("OBJECT"));
    assert!(before.lines.iter().all(|line| !line.contains("[ ]")));
    key(&mut app, KeyCode::Char(' '));
    let one = rendered_tree(&app, tree).unwrap();
    assert!(one.lines[0].starts_with("    OBJECT"));
    assert!(one.lines[1].starts_with("[x] "));
    assert!(one.lines[2].starts_with("[ ] "));
    for (before, after) in before.lines.iter().zip(&one.lines) {
        assert_eq!(before.width(), after.width());
    }
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char(' '));
    assert!(rendered_tree(&app, tree).unwrap().lines[2].starts_with("[x] "));
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE), area);
    let scrolled = rendered_tree(&app, tree).unwrap();
    assert!(scrolled.lines[1].starts_with("[x] "));
    let mut terminal = Terminal::new(TestBackend::new(100, 16)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("2 marked"));
    assert!(rendered.contains("[x]"));
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Left);
    terminal.draw(|frame| render(frame, &app)).unwrap();
    assert!(
        terminal
            .backend()
            .to_string()
            .contains("2 marked (1 hidden)")
    );
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char(' '));
    app.resource_horizontal_scroll = 0;
    assert!(app.marked.is_empty());
    assert_eq!(rendered_tree(&app, tree).unwrap().lines, before.lines);
}

#[test]
fn marks_are_dropped_when_a_resource_is_recreated_or_removed() {
    let mut app = app();
    let with_uid = |child_uid: &str| {
        Snapshot::parse(
            format!(
                r#"{{"object":{{"apiVersion":"v1","kind":"Root","metadata":{{"name":"root","uid":"r"}}}},"children":[{{"object":{{"apiVersion":"v1","kind":"Child","metadata":{{"name":"child","uid":"{child_uid}"}}}}}}]}}"#
            )
            .as_bytes(),
        )
        .unwrap()
    };
    app.apply_snapshot(with_uid("a"));
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(app.marked.len(), 2);
    app.apply_snapshot(with_uid("b"));
    assert_eq!(app.marked.len(), 1);
    assert!(app.marked.keys().all(|identity| identity.kind == "Root"));
    app.apply_resource_not_found();
    assert!(app.marked.is_empty());
}

#[test]
fn ctrl_d_lists_every_marked_resource_and_enter_deletes_them_all() {
    let mut app = app();
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Up);
    app.handle_key(
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        Rect::new(0, 0, 100, 20),
    );
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("Delete 2 resources"));
    assert!(rendered.contains("Root/root"));
    assert!(rendered.contains("Child/child"));
    let UiAction::Delete(targets, DeletePropagation::Foreground) = key(&mut app, KeyCode::Enter)
    else {
        panic!("expected batch delete");
    };
    assert_eq!(
        targets
            .iter()
            .map(|target| target.identity.kind.as_str())
            .collect::<Vec<_>>(),
        ["Root", "Child"]
    );
}

#[test]
fn ctrl_d_without_marks_targets_only_the_highlighted_resource() {
    let mut app = app();
    key(&mut app, KeyCode::Down);
    app.handle_key(
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        Rect::new(0, 0, 100, 20),
    );
    let Some(Modal::Delete { targets, .. }) = &app.modal else {
        panic!("expected delete confirmation");
    };
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].identity.kind, "Child");
}

#[test]
fn batch_delete_respects_read_only_mode() {
    let mut app = app();
    app.config.read_only = true;
    key(&mut app, KeyCode::Char(' '));
    app.handle_key(
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        Rect::new(0, 0, 100, 20),
    );
    assert!(app.modal.is_none());
    assert!(matches!(key(&mut app, KeyCode::Char('p')), UiAction::None));
    assert!(app.status.contains("read-only"));
}

fn app_with_paused_child(root_paused: bool) -> App {
    let mut app = app();
    let root = if root_paused {
        r#","annotations":{"crossplane.io/paused":"true"}"#
    } else {
        ""
    };
    app.apply_snapshot(
        Snapshot::parse(
            format!(
                r#"{{"object":{{"apiVersion":"v1","kind":"Root","metadata":{{"name":"root"{root}}}}},"children":[{{"object":{{"apiVersion":"v1","kind":"Child","metadata":{{"name":"child","annotations":{{"crossplane.io/paused":"true"}}}}}}}}]}}"#
            )
            .as_bytes(),
        )
        .unwrap(),
    );
    app
}

#[test]
fn p_toggles_the_highlighted_resource() {
    let mut app = app();
    let UiAction::SetPaused(targets, true) = key(&mut app, KeyCode::Char('p')) else {
        panic!("expected pause");
    };
    assert_eq!(targets.len(), 1);

    let mut app = app_with_paused_child(false);
    key(&mut app, KeyCode::Down);
    let UiAction::SetPaused(targets, false) = key(&mut app, KeyCode::Char('p')) else {
        panic!("expected unpause");
    };
    assert_eq!(targets[0].identity.kind, "Child");
}

#[test]
fn p_on_uniform_marks_applies_one_direction_and_mixed_marks_ask() {
    let mut app = app_with_paused_child(true);
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char(' '));
    let UiAction::SetPaused(targets, false) = key(&mut app, KeyCode::Char('p')) else {
        panic!("all paused should unpause");
    };
    assert_eq!(targets.len(), 2);

    let mut app = app_with_paused_child(false);
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char(' '));
    assert!(matches!(key(&mut app, KeyCode::Char('p')), UiAction::None));
    assert!(matches!(
        app.modal,
        Some(Modal::PauseChoice {
            paused: 1,
            pause: true,
            ..
        })
    ));
    let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("1 paused, 1 not paused"));
    assert!(rendered.contains("Pause all"));
    key(&mut app, KeyCode::Down);
    let UiAction::SetPaused(targets, false) = key(&mut app, KeyCode::Enter) else {
        panic!("expected unpause all");
    };
    assert_eq!(targets.len(), 2);
}

#[test]
fn batch_completion_clears_marks_and_reports_partial_failures() {
    let mut app = app();
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char(' '));
    let identities: Vec<_> = app.marked.keys().cloned().collect();
    app.active_mutations.extend(identities.iter().cloned());
    assert!(app.finish_batch("Pause 2 resources", &identities, &[], Ok(None)));
    assert!(app.marked.is_empty());
    assert!(app.active_mutations.is_empty());

    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Char(' '));
    let identities: Vec<_> = app.marked.keys().cloned().collect();
    let failed: Vec<_> = identities
        .iter()
        .filter(|identity| identity.kind == "Root")
        .cloned()
        .collect();
    app.active_mutations.extend(identities.iter().cloned());
    assert!(!app.finish_batch(
        "Delete (Foreground) 2 resources",
        &identities,
        &failed,
        Err("1 of 2 resources failed".into())
    ));
    assert_eq!(app.marked.keys().cloned().collect::<Vec<_>>(), failed);
    assert!(app.active_mutations.is_empty());
    assert!(matches!(
        app.modal,
        Some(Modal::Text {
            kind: ContentKind::Error,
            ..
        })
    ));
}

#[test]
fn paused_footer_and_help_describe_space_and_p_only() {
    assert!(HELP_LINES.iter().any(|line| line.contains("Space")));
    assert!(!HELP_LINES.iter().any(|line| line.contains("p / u")));
    let app = app();
    let mut terminal = Terminal::new(TestBackend::new(240, 16)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("Space:mark"));
    assert!(rendered.contains("p:pause"));
    assert!(!rendered.contains("u:unpause"));
    let footer = rendered.find("ctrl-d:delete").unwrap();
    assert!(footer < rendered.find("p:pause").unwrap());
    assert!(rendered.find("p:pause").unwrap() < rendered.find("f:reconcile").unwrap());
    assert!(!rendered.contains("u:upgrade"));
    assert!(rendered.find("::command").unwrap() < rendered.find(":upgrade").unwrap());
    assert!(rendered.find("f:reconcile").unwrap() < rendered.find("d:describe").unwrap());
    assert!(
        HELP_LINES
            .iter()
            .any(|line| line.contains("request immediate reconcile"))
    );
}

fn app_with_finalizers() -> App {
    let mut app = app();
    app.apply_snapshot(
        Snapshot::parse(
            br#"{"object":{"apiVersion":"v1","kind":"Root","metadata":{"name":"root","uid":"r","finalizers":["a.io/one","b.io/two"]}},"children":[
            {"object":{"apiVersion":"v1","kind":"Child","metadata":{"name":"first","uid":"c1","finalizers":["a.io/one"]}}},
            {"object":{"apiVersion":"v1","kind":"Child","metadata":{"name":"second","uid":"c2"}}}]}"#,
        )
        .unwrap(),
    );
    app
}

fn ctrl_x(app: &mut App) {
    app.handle_key(
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
        Rect::new(0, 0, 100, 40),
    );
}

#[test]
fn finalizer_picker_combines_names_with_counts_and_selects_nothing() {
    let mut app = app_with_finalizers();
    for _ in 0..3 {
        key(&mut app, KeyCode::Char(' '));
        key(&mut app, KeyCode::Down);
    }
    ctrl_x(&mut app);
    let Some(Modal::Finalizers {
        targets,
        names,
        selected,
        ..
    }) = &app.modal
    else {
        panic!("expected finalizer picker");
    };
    assert_eq!(targets.len(), 3);
    assert_eq!(
        names,
        &vec![("a.io/one".to_owned(), 2), ("b.io/two".to_owned(), 1)]
    );
    assert!(selected.is_empty());
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("Remove finalizers · 3 resources"));
    assert!(rendered.contains("1 without finalizers will be skipped"));
    assert!(rendered.contains("a.io/one  (2 resources)"));
    assert!(rendered.contains("b.io/two  (1 resource)"));
}

#[test]
fn finalizer_review_is_required_and_lists_exact_changes() {
    let mut app = app_with_finalizers();
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char(' '));
    ctrl_x(&mut app);
    // Enter without a selection does nothing.
    assert!(matches!(key(&mut app, KeyCode::Enter), UiAction::None));
    assert!(matches!(app.modal, Some(Modal::Finalizers { .. })));
    key(&mut app, KeyCode::Char(' '));
    assert!(matches!(key(&mut app, KeyCode::Enter), UiAction::None));
    assert!(matches!(app.modal, Some(Modal::FinalizerReview { .. })));
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("Review finalizer removal"));
    assert!(rendered.contains("Remove: a.io/one"));
    assert!(rendered.contains("Keep:   b.io/two"));
    assert!(rendered.contains("No matching finalizers; skipped"));
    // Back keeps the selection.
    key(&mut app, KeyCode::Backspace);
    assert!(matches!(
        &app.modal,
        Some(Modal::Finalizers { selected, .. }) if selected.len() == 1
    ));
    key(&mut app, KeyCode::Enter);
    let UiAction::RemoveFinalizers(removals) = key(&mut app, KeyCode::Enter) else {
        panic!("expected removals");
    };
    assert_eq!(removals.len(), 2);
    assert_eq!(removals[0].remove, ["a.io/one"]);
    assert_eq!(removals[0].expected, ["a.io/one", "b.io/two"]);
    assert_eq!(removals[1].target.identity.name, "first");
    assert!(app.modal.is_none());
}

#[test]
fn finalizer_review_can_be_cancelled_and_targets_highlighted_without_marks() {
    let mut app = app_with_finalizers();
    ctrl_x(&mut app);
    let Some(Modal::Finalizers { targets, .. }) = &app.modal else {
        panic!("expected picker");
    };
    assert_eq!(targets.len(), 1);
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Enter);
    assert!(matches!(app.modal, Some(Modal::FinalizerReview { .. })));
    key(&mut app, KeyCode::Esc);
    assert!(app.modal.is_none());
}

#[test]
fn finalizer_removal_reports_missing_finalizers_and_respects_read_only() {
    let mut app = app();
    ctrl_x(&mut app);
    assert!(app.modal.is_none());
    assert_eq!(app.status, "Selected resource has no finalizers");
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char(' '));
    ctrl_x(&mut app);
    assert_eq!(app.status, "The selected resources have no finalizers");
    let mut app = app_with_finalizers();
    app.config.read_only = true;
    ctrl_x(&mut app);
    assert!(app.modal.is_none());
    assert!(app.status.contains("read-only"));
}

#[test]
fn clicking_a_checkbox_toggles_that_resource_without_moving_the_cursor() {
    let mut app = app();
    let area = Rect::new(0, 0, 80, 12);
    let body = Block::default()
        .borders(Borders::ALL)
        .inner(resource_tree_area(area).unwrap());
    let click = |app: &mut App, column: u16, row: u16| {
        for kind in [
            MouseEventKind::Down(MouseButton::Left),
            MouseEventKind::Up(MouseButton::Left),
        ] {
            assert!(
                app.handle_mouse(
                    MouseEvent {
                        kind,
                        column,
                        row,
                        modifiers: KeyModifiers::NONE,
                    },
                    area,
                )
                .is_none()
            );
        }
    };
    // Without marks there is no checkbox column; the click only selects the row.
    click(&mut app, body.x + 1, body.y + 2);
    assert!(app.marked.is_empty());
    assert_eq!(app.selection.visible_index, 1);
    app.set_selection(0);

    key(&mut app, KeyCode::Char(' '));
    assert_eq!(app.marked.len(), 1);
    click(&mut app, body.x + 1, body.y + 2);
    assert_eq!(app.marked.len(), 2);
    assert_eq!(app.selection.visible_index, 0);
    assert!(app.tree_selection.is_none());
    click(&mut app, body.x + 1, body.y + 2);
    assert_eq!(app.marked.len(), 1);
    click(&mut app, body.x + 1, body.y + 1);
    assert!(app.marked.is_empty());

    // Outside the checkbox cells a click still just selects the row.
    key(&mut app, KeyCode::Char(' '));
    click(&mut app, body.x + 6, body.y + 2);
    assert_eq!(app.marked.len(), 1);
    assert_eq!(app.selection.visible_index, 1);
}

fn revision(name: &str, composition: &str, number: i64, kind: &str) -> CompositionRevisionInfo {
    CompositionRevisionInfo {
        name: name.into(),
        composition: composition.into(),
        number,
        composite_kind: kind.into(),
        composite_group: "example.io".into(),
    }
}

fn xr(name: &str, kind: &str, spec: &str) -> String {
    format!(
        r#"{{"object":{{"apiVersion":"example.io/v1","kind":"{kind}","metadata":{{"name":"{name}","uid":"{name}-uid"}},"spec":{spec}}}"#
    )
}

/// Root (manual, revision 1 of 3) with children: manual at latest, automatic, plain.
fn app_for_upgrades() -> App {
    let manual = |revision: &str| {
        format!(
            r#"{{"compositionRef":{{"name":"db"}},"compositionUpdatePolicy":"Manual","compositionRevisionRef":{{"name":"{revision}"}}}}"#
        )
    };
    let trace = format!(
        r#"{},"children":[{}}},{}}},{{"object":{{"apiVersion":"v1","kind":"Plain","metadata":{{"name":"plain","uid":"p"}}}}}}]}}"#,
        xr("old", "XDb", &manual("db-1")),
        xr("current", "XDb", &manual("db-3")),
        xr(
            "auto",
            "XDb",
            r#"{"compositionRef":{"name":"db"},"compositionRevisionRef":{"name":"db-1"}}"#
        ),
    );
    let mut app = app();
    app.apply_snapshot(Snapshot::parse(trace.as_bytes()).unwrap());
    app.revisions = RevisionDiscovery::loaded(upgrade::RevisionIndex::new(vec![
        revision("db-1", "db", 1, "XDb"),
        revision("db-3", "db", 3, "XDb"),
        revision("db-2", "db", 2, "XDb"),
        revision("other-9", "db", 9, "Other"),
    ]));
    app
}

#[test]
fn upgrade_info_goes_to_the_highest_revision_of_the_same_composition() {
    use upgrade::{UpgradeState, upgrade_info};
    let app = app_for_upgrades();
    let nodes = &app.snapshot.as_ref().unwrap().nodes;
    let state = |index: usize| upgrade_info(&nodes[index], &app.revisions);
    let old = state(0);
    assert_eq!(old.state, UpgradeState::Available);
    assert_eq!((old.current, old.latest), (Some(1), Some(3)));
    let plan = old.plan.unwrap();
    assert_eq!(
        (plan.from_name.as_str(), plan.to_name.as_str()),
        ("db-1", "db-3")
    );
    assert_eq!(state(1).state, UpgradeState::UpToDate);
    assert!(state(1).plan.is_none());
    assert_eq!(state(2).state, UpgradeState::Automatic);
    assert_eq!(state(3).state, UpgradeState::NotApplicable);
    assert_eq!(
        upgrade_info(&nodes[0], &RevisionDiscovery::loading()).state,
        UpgradeState::Checking
    );
    assert_eq!(
        upgrade_info(&nodes[0], &RevisionDiscovery::failed()).state,
        UpgradeState::Failed
    );
}

#[test]
fn upgrade_info_handles_missing_unknown_and_crossplane_v2_layouts() {
    use upgrade::{UpgradeState, upgrade_info};
    let mut app = app_for_upgrades();
    let trace = format!(
        r#"{},"children":[{}}},{}}}]}}"#,
        xr(
            "none",
            "XDb",
            r#"{"crossplane":{"compositionRef":{"name":"db"},"compositionUpdatePolicy":"Manual"}}"#
        ),
        xr(
            "unknown",
            "XDb",
            r#"{"compositionRef":{"name":"db"},"compositionUpdatePolicy":"Manual","compositionRevisionRef":{"name":"gone"}}"#
        ),
        xr(
            "v2",
            "XDb",
            r#"{"crossplane":{"compositionRef":{"name":"db"},"compositionUpdatePolicy":"Manual","compositionRevisionRef":{"name":"db-2"}}}"#
        ),
    );
    app.apply_snapshot(Snapshot::parse(trace.as_bytes()).unwrap());
    let nodes = &app.snapshot.as_ref().unwrap().nodes;
    let state = |index: usize| upgrade_info(&nodes[index], &app.revisions);
    assert_eq!(state(0).state, UpgradeState::NoRevision);
    assert_eq!(state(1).state, UpgradeState::UnknownRevision);
    assert_eq!(state(2).state, UpgradeState::Available);
    assert_eq!(state(2).plan.unwrap().to_number, 3);
}

#[test]
fn u_opens_the_upgrade_view_and_renders_colored_states() {
    let mut app = app_for_upgrades();
    let mut terminal = Terminal::new(TestBackend::new(140, 20)).unwrap();
    assert!(matches!(
        open_upgrade(&mut app),
        UiAction::DiscoverRevisions
    ));
    assert!(app.upgrade_view);
    let generation = app.revisions.begin_refresh();
    app.apply_revisions(
        generation,
        Ok(vec![
            revision("db-1", "db", 1, "XDb"),
            revision("db-3", "db", 3, "XDb"),
        ]),
    );
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let rendered = terminal.backend().to_string();
    assert!(
        rendered.contains("Upgrade revisions (4/4) · 1 available"),
        "{rendered}"
    );
    for header in ["CURRENT", "LATEST", "UPGRADE"] {
        assert!(rendered.contains(header));
    }
    assert!(!rendered.contains("SYNCED"));
    assert!(rendered.contains("Available ↑"));
    assert!(rendered.contains("Up to date"));
    assert!(rendered.contains("Automatic"));
    assert!(rendered.contains("Space:mark"));

    let tree = resource_tree_area(Rect::new(0, 0, 140, 20)).unwrap();
    let rendered_tree = rendered_tree(&app, tree).unwrap();
    let (start, _) = rendered_tree.upgrade_range.unwrap();
    let offset = rendered_tree.horizontal.offset;
    let cell = |terminal: &Terminal<TestBackend>, row: u16| {
        terminal
            .backend()
            .buffer()
            .cell((1 + (start - offset) as u16, 3 + row))
            .unwrap()
            .clone()
    };
    // Row 0 is selected and keeps the selection colours; the others use theme colours.
    assert_eq!(cell(&terminal, 0).fg, app.theme.palette.base);
    assert_eq!(cell(&terminal, 1).symbol(), "U");
    assert_eq!(cell(&terminal, 1).fg, app.theme.palette.green);
    assert_eq!(cell(&terminal, 2).fg, app.theme.palette.overlay1);
    app.set_selection(1);
    terminal.draw(|frame| render(frame, &app)).unwrap();
    assert_eq!(cell(&terminal, 0).symbol(), "A");
    assert_eq!(cell(&terminal, 0).fg, app.theme.palette.peach);
}

#[test]
fn upgrade_view_marks_only_eligible_visible_resources() {
    let mut app = app_for_upgrades();
    open_upgrade(&mut app);
    app.revisions = app_for_upgrades().revisions;
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char(' '));
    assert!(app.upgrade_marked.is_empty());
    assert!(app.status.contains("No revision upgrade"));
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(app.upgrade_marked.len(), 1);
    assert!(app.marked.is_empty());
    key(&mut app, KeyCode::Char(' '));
    assert!(app.upgrade_marked.is_empty());

    // `a` marks visible eligible rows only: collapsing hides the children.
    key(&mut app, KeyCode::Char('a'));
    assert_eq!(app.upgrade_marked.len(), 1);
    app.upgrade_marked.clear();
    app.apply_snapshot(
        Snapshot::parse(
            format!(
                r#"{},"children":[{}}}]}}"#,
                xr(
                    "top",
                    "XDb",
                    r#"{"compositionRef":{"name":"db"},"compositionUpdatePolicy":"Manual","compositionRevisionRef":{"name":"db-1"}}"#
                ),
                xr(
                    "inner",
                    "XDb",
                    r#"{"compositionRef":{"name":"db"},"compositionUpdatePolicy":"Manual","compositionRevisionRef":{"name":"db-2"}}"#
                ),
            )
            .as_bytes(),
        )
        .unwrap(),
    );
    key(&mut app, KeyCode::Left);
    key(&mut app, KeyCode::Char('a'));
    assert_eq!(app.upgrade_marked.len(), 1);
    assert!(
        app.upgrade_marked
            .keys()
            .all(|identity| identity.name == "top")
    );
}

#[test]
fn upgrade_review_applies_only_after_enter_and_respects_read_only() {
    let mut app = app_for_upgrades();
    open_upgrade(&mut app);
    app.revisions = app_for_upgrades().revisions;
    // Nothing marked: the highlighted upgrade is reviewed.
    assert!(matches!(key(&mut app, KeyCode::Enter), UiAction::None));
    let Some(Modal::UpgradeReview { plans }) = &app.modal else {
        panic!("expected review");
    };
    assert_eq!(plans.len(), 1);
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("Apply revision update"));
    assert!(rendered.contains("XDb/old"));
    assert!(rendered.contains("1 -> 3  db"));
    assert!(rendered.contains("Only compositionRevisionRef changes"));
    key(&mut app, KeyCode::Esc);
    assert!(app.modal.is_none());
    assert!(app.upgrade_view);
    key(&mut app, KeyCode::Enter);
    let UiAction::UpgradeRevisions(plans) = key(&mut app, KeyCode::Enter) else {
        panic!("expected upgrade action");
    };
    assert_eq!(plans[0].to_name, "db-3");

    app.config.read_only = true;
    key(&mut app, KeyCode::Enter);
    assert!(app.modal.is_none());
    assert!(app.status.contains("read-only"));
}

#[test]
fn upgrade_view_blocks_other_mutations_and_exits_with_u_or_esc() {
    let mut app = app_for_upgrades();
    open_upgrade(&mut app);
    for code in [KeyCode::Char('p'), KeyCode::Char('e')] {
        assert!(matches!(key(&mut app, code), UiAction::None));
        assert!(app.status.contains("Not available in the upgrade view"));
    }
    app.handle_key(
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        Rect::new(0, 0, 100, 20),
    );
    assert!(app.modal.is_none());
    // Navigation and `r` still work.
    key(&mut app, KeyCode::Down);
    assert_eq!(app.selection.visible_index, 1);
    assert!(matches!(
        key(&mut app, KeyCode::Char('r')),
        UiAction::Refresh
    ));
    key(&mut app, KeyCode::Esc);
    assert!(!app.upgrade_view);

    open_upgrade(&mut app);
    app.revisions = app_for_upgrades().revisions;
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Esc);
    assert!(app.upgrade_view && app.upgrade_marked.is_empty());
    key(&mut app, KeyCode::Esc);
    assert!(!app.upgrade_view);
}

#[test]
fn upgrade_completion_clears_only_upgrade_marks() {
    let mut app = app_for_upgrades();
    key(&mut app, KeyCode::Char(' '));
    let identity = app.marked.keys().next().cloned().unwrap();
    app.upgrade_marked.insert(identity.clone(), None);
    app.active_mutations.insert(identity.clone());
    assert!(app.finish_action("Upgrade revision", Some(identity.clone()), Ok(None)));
    assert!(app.upgrade_marked.is_empty());
    assert_eq!(app.marked.len(), 1);
    app.finish_action("Pause", Some(identity), Ok(None));
    assert!(app.marked.is_empty());
}

#[test]
fn upgrade_checkbox_click_toggles_only_eligible_rows() {
    let mut app = app_for_upgrades();
    open_upgrade(&mut app);
    app.revisions = app_for_upgrades().revisions;
    key(&mut app, KeyCode::Char('a'));
    assert_eq!(app.upgrade_marked.len(), 1);
    let area = Rect::new(0, 0, 100, 20);
    let body = Block::default()
        .borders(Borders::ALL)
        .inner(resource_tree_area(area).unwrap());
    for row in [2, 1] {
        for kind in [
            MouseEventKind::Down(MouseButton::Left),
            MouseEventKind::Up(MouseButton::Left),
        ] {
            app.handle_mouse(
                MouseEvent {
                    kind,
                    column: body.x + 1,
                    row: body.y + row,
                    modifiers: KeyModifiers::NONE,
                },
                area,
            );
        }
        // Row 2 is not eligible, so the mark count only changes for row 1.
        assert_eq!(app.upgrade_marked.len(), usize::from(row == 2));
    }
}

#[test]
fn upgrade_review_keeps_revisions_visible_for_long_names() {
    let long_name = format!("e2e-persistent-{}-tail", "dashboard-".repeat(8));
    let composition = format!("fdc-{}.dai-platform.nn.nl", "account-usage-".repeat(4));
    let plan = upgrade::UpgradePlan {
        target: Target {
            identity: Identity {
                group: "dai-platform.nn.nl".into(),
                version: "v1".into(),
                kind: "XFinOpsDashboardContent".into(),
                namespace: None,
                name: long_name,
            },
            expected_uid: None,
        },
        composition,
        from_name: "a".into(),
        from_number: 10,
        to_name: "b".into(),
        to_number: 11,
    };
    let mut app = app();
    app.modal = Some(Modal::UpgradeReview {
        plans: vec![plan.clone(), plan],
    });
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("10 -> 11"), "{rendered}");
    assert!(
        rendered.contains("XFinOpsDashboardContent/e2e-"),
        "{rendered}"
    );
    assert!(rendered.contains("-tail"), "{rendered}");
    assert!(rendered.contains("dai-platform.nn.nl"), "{rendered}");
    assert!(rendered.contains('…'));
}

#[test]
fn truncate_middle_keeps_both_ends_within_width() {
    assert_eq!(truncate_middle("short", 10), "short");
    let truncated = truncate_middle("abcdefghijklmnopqrstuvwxyz", 11);
    assert_eq!(truncated.width(), 11);
    assert!(truncated.starts_with("abcde") && truncated.ends_with("vwxyz"));
    assert!(truncate_middle("界界界界界界", 7).width() <= 7);
    assert_eq!(truncate_middle("abc", 1), "…");
}

#[test]
fn upgrade_view_uses_a_peach_border_and_normal_view_keeps_lavender() {
    let mut app = app_for_upgrades();
    let mut terminal = Terminal::new(TestBackend::new(100, 16)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    assert_eq!(
        terminal.backend().buffer().cell((0, 1)).unwrap().fg,
        app.theme.palette.lavender
    );
    open_upgrade(&mut app);
    terminal.draw(|frame| render(frame, &app)).unwrap();
    assert_eq!(
        terminal.backend().buffer().cell((0, 1)).unwrap().fg,
        app.theme.palette.peach
    );
    key(&mut app, KeyCode::Esc);
    terminal.draw(|frame| render(frame, &app)).unwrap();
    assert_eq!(
        terminal.backend().buffer().cell((0, 1)).unwrap().fg,
        app.theme.palette.lavender
    );
}

fn discovered(app: &mut App) {
    let generation = app.revisions.begin_refresh();
    app.apply_revisions(
        generation,
        Ok(vec![
            revision("db-1", "db", 1, "XDb"),
            revision("db-3", "db", 3, "XDb"),
        ]),
    );
}

fn first_state(app: &App) -> upgrade::UpgradeState {
    upgrade::upgrade_info(&app.snapshot.as_ref().unwrap().nodes[0], &app.revisions).state
}

#[test]
fn reentering_the_upgrade_view_shows_cached_results_while_refreshing() {
    use upgrade::UpgradeState;
    let mut app = app_for_upgrades();
    app.revisions = RevisionDiscovery::default();
    assert!(matches!(
        open_upgrade(&mut app),
        UiAction::DiscoverRevisions
    ));
    assert_eq!(first_state(&app), UpgradeState::Checking);
    discovered(&mut app);
    assert_eq!(first_state(&app), UpgradeState::Available);
    key(&mut app, KeyCode::Esc);
    assert!(!app.upgrade_view);

    assert!(matches!(
        open_upgrade(&mut app),
        UiAction::DiscoverRevisions
    ));
    // The refresh has not returned yet, but the previous result is still used.
    app.revisions.begin_refresh();
    assert!(app.revisions.is_refreshing());
    assert_eq!(first_state(&app), UpgradeState::Available);
    let mut terminal = Terminal::new(TestBackend::new(140, 20)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("1 available · refreshing"), "{rendered}");
    assert!(rendered.contains("Available ↑"));
    assert!(!rendered.contains("Checking"));
}

#[test]
fn stale_discovery_results_are_ignored() {
    let mut app = app_for_upgrades();
    app.revisions = RevisionDiscovery::default();
    let old = app.revisions.begin_refresh();
    let new = app.revisions.begin_refresh();
    app.apply_revisions(old, Ok(vec![revision("db-1", "db", 1, "XDb")]));
    assert!(app.revisions.is_refreshing());
    assert!(!app.revisions.has_cache());
    app.apply_revisions(new, Ok(vec![revision("db-1", "db", 1, "XDb")]));
    assert!(!app.revisions.is_refreshing());
    assert!(app.revisions.has_cache());
}

#[test]
fn failed_refresh_keeps_cached_results_and_says_so() {
    use upgrade::UpgradeState;
    let mut app = app_for_upgrades();
    app.revisions = RevisionDiscovery::default();
    open_upgrade(&mut app);
    discovered(&mut app);
    let generation = app.revisions.begin_refresh();
    app.apply_revisions(generation, Err("connection refused".into()));
    assert_eq!(first_state(&app), UpgradeState::Available);
    assert!(!app.revisions.is_refreshing());
    assert_eq!(app.revisions.error(), Some("connection refused"));
    assert!(app.status.contains("showing cached results"));
    let mut terminal = Terminal::new(TestBackend::new(140, 20)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    assert!(
        terminal
            .backend()
            .to_string()
            .contains("cached, refresh failed")
    );

    // Without a cache a failure shows as unavailable.
    let mut app = app_for_upgrades();
    app.revisions = RevisionDiscovery::default();
    let generation = app.revisions.begin_refresh();
    app.apply_revisions(generation, Err("boom".into()));
    assert_eq!(first_state(&app), UpgradeState::Failed);
}

#[test]
fn an_open_review_is_not_changed_by_a_background_refresh() {
    let mut app = app_for_upgrades();
    open_upgrade(&mut app);
    key(&mut app, KeyCode::Enter);
    let Some(Modal::UpgradeReview { plans }) = &app.modal else {
        panic!("expected review");
    };
    assert_eq!(plans[0].to_name, "db-3");
    let generation = app.revisions.begin_refresh();
    app.apply_revisions(
        generation,
        Ok(vec![
            revision("db-1", "db", 1, "XDb"),
            revision("db-3", "db", 3, "XDb"),
            revision("db-4", "db", 4, "XDb"),
        ]),
    );
    let UiAction::UpgradeRevisions(plans) = key(&mut app, KeyCode::Enter) else {
        panic!("expected upgrade action");
    };
    assert_eq!(plans[0].to_name, "db-3");
}

fn upgrade_plan_for_first(app: &App) -> upgrade::UpgradePlan {
    app.upgrade_info_for(&app.snapshot.as_ref().unwrap().nodes[0])
        .plan
        .unwrap()
}

#[test]
fn upgrading_shows_immediately_and_clears_when_a_trace_reflects_it() {
    use upgrade::UpgradeState;
    let mut app = app_for_upgrades();
    open_upgrade(&mut app);
    let plan = upgrade_plan_for_first(&app);
    app.begin_upgrades(std::slice::from_ref(&plan));
    assert_eq!(first_state(&app), UpgradeState::Upgrading);
    // Revision columns stay, and an in-flight resource cannot be marked again.
    let info = app.upgrade_info_for(&app.snapshot.as_ref().unwrap().nodes[0]);
    assert_eq!((info.current, info.latest), (Some(1), Some(3)));
    assert!(info.plan.is_none());
    key(&mut app, KeyCode::Char(' '));
    assert!(app.upgrade_marked.is_empty());

    let mut terminal = Terminal::new(TestBackend::new(140, 20)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("Upgrading…"), "{rendered}");
    assert!(rendered.contains("0 available"), "{rendered}");
    app.set_selection(1);
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let tree = resource_tree_area(Rect::new(0, 0, 140, 20)).unwrap();
    let (start, _) = rendered_tree(&app, tree).unwrap().upgrade_range.unwrap();
    let offset = rendered_tree(&app, tree).unwrap().horizontal.offset;
    let cell = terminal
        .backend()
        .buffer()
        .cell((1 + (start - offset) as u16, 3))
        .unwrap();
    assert_eq!(cell.symbol(), "U");
    assert_eq!(cell.fg, app.theme.palette.sky);

    // Success keeps showing Upgrading until a trace shows the new reference.
    app.active_mutations.insert(plan.target.identity.clone());
    assert!(app.finish_action(
        "Upgrade revision",
        Some(plan.target.identity.clone()),
        Ok(None)
    ));
    assert_eq!(first_state(&app), UpgradeState::Upgrading);
    let manual = |revision: &str| {
        format!(
            r#"{{"compositionRef":{{"name":"db"}},"compositionUpdatePolicy":"Manual","compositionRevisionRef":{{"name":"{revision}"}}}}"#
        )
    };
    let trace = format!(r#"{}}}"#, xr("old", "XDb", &manual("db-1")));
    app.apply_snapshot(Snapshot::parse(trace.as_bytes()).unwrap());
    assert_eq!(first_state(&app), UpgradeState::Upgrading);
    let trace = format!(r#"{}}}"#, xr("old", "XDb", &manual("db-3")));
    app.apply_snapshot(Snapshot::parse(trace.as_bytes()).unwrap());
    assert_eq!(first_state(&app), UpgradeState::UpToDate);
}

#[test]
fn a_failed_upgrade_returns_to_available() {
    use upgrade::UpgradeState;
    let mut app = app_for_upgrades();
    open_upgrade(&mut app);
    let plan = upgrade_plan_for_first(&app);
    let identity = plan.target.identity.clone();
    app.begin_upgrades(std::slice::from_ref(&plan));
    app.active_mutations.insert(identity.clone());
    assert!(!app.finish_action(
        "Upgrade revision",
        Some(identity.clone()),
        Err("boom".into())
    ));
    assert_eq!(first_state(&app), UpgradeState::Available);

    // In a batch only the failed resources go back to Available.
    let other = app.snapshot.as_ref().unwrap().nodes[1].identity.clone();
    let mut second = plan.clone();
    second.target.identity = other.clone();
    app.begin_upgrades(&[plan, second]);
    let identities = [identity, other.clone()];
    app.active_mutations.extend(identities.iter().cloned());
    app.finish_batch(
        "Upgrade revision 2 resources",
        &identities,
        &identities[..1],
        Err("1 of 2 failed".into()),
    );
    assert_eq!(first_state(&app), UpgradeState::Available);
    assert!(app.revisions.pending_contains(&other));
}

#[test]
fn f_requests_reconcile_and_ctrl_f_starts_find() {
    let mut app = app();
    key(&mut app, KeyCode::Down);
    let UiAction::Reconcile(targets) = key(&mut app, KeyCode::Char('f')) else {
        panic!("expected reconcile");
    };
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].identity.kind, "Child");
    // Shift+F is unbound.
    let shifted = app.handle_key(
        KeyEvent::new(KeyCode::Char('F'), KeyModifiers::SHIFT),
        Rect::new(0, 0, 100, 20),
    );
    assert!(matches!(shifted, UiAction::None));

    let mut marked = app_for_upgrades();
    key(&mut marked, KeyCode::Char(' '));
    key(&mut marked, KeyCode::Down);
    key(&mut marked, KeyCode::Char(' '));
    let UiAction::Reconcile(targets) = key(&mut marked, KeyCode::Char('f')) else {
        panic!("expected reconcile");
    };
    assert_eq!(targets.len(), 2);

    let mut finder = self::app();
    let action = finder.handle_key(
        KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
        Rect::new(0, 0, 100, 20),
    );
    assert!(matches!(action, UiAction::None));
    assert_eq!(finder.mode, InputMode::Find);
}

#[test]
fn ctrl_f_no_longer_pages_down_but_page_down_does() {
    let mut app = app();
    let area = Rect::new(0, 0, 100, 20);
    app.handle_key(
        KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
        area,
    );
    assert_eq!(app.mode, InputMode::Find);
    app.mode = InputMode::Normal;
    app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), area);
    assert_eq!(app.selection.visible_index, 1);
}

#[test]
fn reconcile_is_blocked_in_read_only_and_the_upgrade_view() {
    let mut app = app();
    app.config.read_only = true;
    assert!(matches!(key(&mut app, KeyCode::Char('f')), UiAction::None));
    assert!(app.status.contains("read-only"));

    let mut app = app_for_upgrades();
    open_upgrade(&mut app);
    assert!(matches!(key(&mut app, KeyCode::Char('f')), UiAction::None));
    assert!(app.status.contains("Not available in the upgrade view"));
}

#[test]
fn reconcile_completion_clears_only_the_normal_marks() {
    let mut app = app();
    key(&mut app, KeyCode::Char(' '));
    let identity = app.marked.keys().next().cloned().unwrap();
    app.active_mutations.insert(identity.clone());
    assert!(app.finish_action("Request reconcile", Some(identity), Ok(None)));
    assert!(app.marked.is_empty());
    assert_eq!(app.status, "Request reconcile succeeded");
}

fn right_click(app: &mut App, area: Rect, row_offset: u16) {
    let body = Block::default()
        .borders(Borders::ALL)
        .inner(resource_tree_area(area).unwrap());
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: body.x + 8,
            row: body.y + row_offset,
            modifiers: KeyModifiers::NONE,
        },
        area,
    );
}

fn menu_labels(app: &App) -> Vec<&'static str> {
    let Some(Modal::ContextMenu { items, .. }) = &app.modal else {
        panic!("expected context menu");
    };
    items.iter().map(|(_, label)| *label).collect()
}

fn click_menu_label(app: &mut App, area: Rect, label: &str) -> Option<MouseAction> {
    let Some(Modal::ContextMenu {
        column, row, items, ..
    }) = &app.modal
    else {
        panic!("expected context menu");
    };
    let index = items.iter().position(|(_, text)| *text == label).unwrap();
    let menu = context_menu_area_with(area, *column, *row, items);
    let inner = Block::default().borders(Borders::ALL).inner(menu);
    let event = |kind| MouseEvent {
        kind,
        column: inner.x,
        row: inner.y + index as u16,
        modifiers: KeyModifiers::NONE,
    };
    app.handle_mouse(event(MouseEventKind::Down(MouseButton::Left)), area);
    app.handle_mouse(event(MouseEventKind::Up(MouseButton::Left)), area)
}

#[test]
fn upgrade_view_menu_has_only_inspection_and_upgrade_actions() {
    let area = Rect::new(0, 0, 100, 20);
    let mut app = app_for_upgrades();
    open_upgrade(&mut app);
    // Row 1 is the first resource, which has an upgrade available.
    right_click(&mut app, area, 1);
    assert_eq!(
        menu_labels(&app),
        [
            "YAML",
            "Status",
            "Events",
            "Describe",
            "────────────",
            "Mark upgrade",
            "Review upgrade"
        ]
    );
    for removed in ["Edit", "Pause", "Unpause", "Reconcile"] {
        assert!(!menu_labels(&app).contains(&removed));
    }
    let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    assert!(terminal.backend().to_string().contains("Review upgrade"));

    // Without an available upgrade (up to date) only inspection remains.
    app.modal = None;
    right_click(&mut app, area, 2);
    assert_eq!(menu_labels(&app), ["YAML", "Status", "Events", "Describe"]);
    // The separator never dispatches anything.
    app.modal = None;
    right_click(&mut app, area, 1);
    assert!(click_menu_label(&mut app, area, "────────────").is_none());
    assert!(app.modal.is_none());
}

#[test]
fn upgrade_menu_marks_unmarks_and_reviews_only_the_clicked_resource() {
    let area = Rect::new(0, 0, 100, 20);
    let mut app = app_for_upgrades();
    open_upgrade(&mut app);
    right_click(&mut app, area, 1);
    assert!(click_menu_label(&mut app, area, "Mark upgrade").is_none());
    assert_eq!(app.upgrade_marked.len(), 1);
    assert!(app.modal.is_none());

    right_click(&mut app, area, 1);
    assert!(menu_labels(&app).contains(&"Unmark upgrade"));
    assert!(!menu_labels(&app).contains(&"Mark upgrade"));
    assert!(click_menu_label(&mut app, area, "Unmark upgrade").is_none());
    assert!(app.upgrade_marked.is_empty());

    // Review applies to the clicked resource even when others are marked.
    key(&mut app, KeyCode::Char('a'));
    right_click(&mut app, area, 1);
    assert!(click_menu_label(&mut app, area, "Review upgrade").is_none());
    let Some(Modal::UpgradeReview { plans }) = &app.modal else {
        panic!("expected review");
    };
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].target.identity.name, "old");
}

#[test]
fn upgrade_menu_omits_review_in_read_only_mode_but_keeps_marking() {
    let area = Rect::new(0, 0, 100, 20);
    let mut app = app_for_upgrades();
    app.config.read_only = true;
    open_upgrade(&mut app);
    right_click(&mut app, area, 1);
    let labels = menu_labels(&app);
    assert!(labels.contains(&"Mark upgrade"));
    assert!(!labels.contains(&"Review upgrade"));
    // A menu opened earlier still refuses to review once read-only applies.
    app.modal = None;
    app.config.read_only = false;
    right_click(&mut app, area, 1);
    app.config.read_only = true;
    click_menu_label(&mut app, area, "Review upgrade");
    assert!(app.modal.is_none());
    assert!(app.status.contains("read-only"));
}

#[test]
fn normal_view_menu_is_unchanged_by_the_upgrade_menu() {
    let area = Rect::new(0, 0, 100, 20);
    let mut app = app_for_upgrades();
    right_click(&mut app, area, 1);
    assert_eq!(
        menu_labels(&app),
        [
            "YAML",
            "Status",
            "Events",
            "Describe",
            "Edit",
            "Pause",
            "Reconcile"
        ]
    );
}

#[test]
fn context_menu_pads_the_longest_label_on_both_sides() {
    let area = Rect::new(0, 0, 100, 20);
    let mut app = app_for_upgrades();
    open_upgrade(&mut app);
    right_click(&mut app, area, 1);
    let Some(Modal::ContextMenu {
        column, row, items, ..
    }) = &app.modal
    else {
        panic!("expected context menu");
    };
    let menu = context_menu_area_with(area, *column, *row, items);
    let inner = Block::default().borders(Borders::ALL).inner(menu);
    let widest = items.iter().map(|(_, label)| label.width()).max().unwrap();
    // One leading and one trailing space around the longest label.
    assert_eq!(usize::from(inner.width), widest + 2);
    let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer();
    let index = items
        .iter()
        .position(|(_, label)| *label == "Review upgrade")
        .unwrap();
    let y = inner.y + index as u16;
    assert_eq!(buffer.cell((inner.right() - 1, y)).unwrap().symbol(), " ");
    assert_eq!(buffer.cell((inner.right() - 2, y)).unwrap().symbol(), "e");
}

#[test]
fn ctrl_j_and_ctrl_k_page_while_plain_j_and_k_move_one_row() {
    let mut app = app();
    let area = Rect::new(0, 0, 100, 20);
    let press = |app: &mut App, code, modifiers| {
        app.handle_key(KeyEvent::new(code, modifiers), area);
    };
    press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
    assert_eq!(app.selection.visible_index, 1);
    press(&mut app, KeyCode::Char('k'), KeyModifiers::NONE);
    assert_eq!(app.selection.visible_index, 0);
    // A page is larger than this two-row tree, so paging reaches the last/first row.
    press(&mut app, KeyCode::Char('j'), KeyModifiers::CONTROL);
    assert_eq!(app.selection.visible_index, 1);
    press(&mut app, KeyCode::Char('k'), KeyModifiers::CONTROL);
    assert_eq!(app.selection.visible_index, 0);
    // Ctrl+J is not Enter: no details or modal opens.
    assert!(app.modal.is_none());
    // Ctrl+B no longer pages.
    press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
    press(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL);
    assert_eq!(app.selection.visible_index, 1);
}

#[test]
fn ctrl_j_and_ctrl_k_page_a_longer_tree_by_the_page_size() {
    let mut app = app();
    let children = (0..40)
        .map(|index| {
            format!(
                r#"{{"object":{{"apiVersion":"v1","kind":"Child","metadata":{{"name":"c{index}"}}}}}}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let trace = format!(
        r#"{{"object":{{"apiVersion":"v1","kind":"Root","metadata":{{"name":"root"}}}},"children":[{children}]}}"#
    );
    app.apply_snapshot(Snapshot::parse(trace.as_bytes()).unwrap());
    let area = Rect::new(0, 0, 100, 20);
    let page = usize::from(area.height - 6);
    app.handle_key(
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL),
        area,
    );
    assert_eq!(app.selection.visible_index, page);
    app.handle_key(
        KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
        area,
    );
    assert_eq!(app.selection.visible_index, 0);
}

/// Opens the upgrade view the way a user does: `:upgrade` in the command palette.
fn open_upgrade(app: &mut App) -> UiAction {
    key(app, KeyCode::Char(':'));
    for character in "upgrade".chars() {
        key(app, KeyCode::Char(character));
    }
    key(app, KeyCode::Enter)
}

#[test]
fn upgrade_command_opens_the_view_and_u_is_unbound() {
    let mut app = app_for_upgrades();
    assert!(matches!(key(&mut app, KeyCode::Char('u')), UiAction::None));
    assert!(!app.upgrade_view);

    // The command shows in the palette and matches by prefix.
    key(&mut app, KeyCode::Char(':'));
    key(&mut app, KeyCode::Char('u'));
    key(&mut app, KeyCode::Char('p'));
    let entries = app.palette_entries();
    assert!(entries.iter().any(|entry| entry.label == "upgrade"));
    key(&mut app, KeyCode::Esc);

    assert!(matches!(
        open_upgrade(&mut app),
        UiAction::DiscoverRevisions
    ));
    assert!(app.upgrade_view);
    assert_eq!(app.mode, InputMode::Normal);
}

#[test]
fn upgrade_command_inside_the_view_keeps_marks_and_does_not_rediscover() {
    let mut app = app_for_upgrades();
    open_upgrade(&mut app);
    app.revisions = app_for_upgrades().revisions;
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(app.upgrade_marked.len(), 1);
    assert!(matches!(open_upgrade(&mut app), UiAction::None));
    assert!(app.upgrade_view);
    assert_eq!(app.upgrade_marked.len(), 1);
    // `u` does nothing in the view either, and Esc still goes back (marks first).
    assert!(matches!(key(&mut app, KeyCode::Char('u')), UiAction::None));
    key(&mut app, KeyCode::Esc);
    assert!(app.upgrade_view && app.upgrade_marked.is_empty());
    key(&mut app, KeyCode::Esc);
    assert!(!app.upgrade_view);
}
