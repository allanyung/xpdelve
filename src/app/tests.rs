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
    app.collapse_or_parent();
    assert_eq!(app.visible().len(), 1);
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
        matches!(&app.modal, Some(Modal::Text { content, vertical_scroll, .. }) if *vertical_scroll > 0 && content.contains("needle\\x1b[31m\\u{202E}") && !content.contains('\u{1b}') && !content.contains('\u{202e}'))
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
        matches!(&app.modal, Some(Modal::Text { query, vertical_scroll, .. }) if query == "needle" && *vertical_scroll > 40)
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
        wrapped,
        ..
    }) = &app.modal
    else {
        panic!("expected resource problem details");
    };
    assert_eq!(title, "Resource details: Child/child");
    assert!(content.contains("Error: cannot connect"));
    assert!(content.contains("Ready=False\nWaiting: a long diagnostic"));
    assert_eq!(*kind, ContentKind::Error);
    assert!(*wrapped);
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
        app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE), area),
        UiAction::None
    ));
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
    let Some(Modal::Text {
        content, wrapped, ..
    }) = &app.modal
    else {
        panic!("expected error modal");
    };
    assert!(*wrapped);
    assert!(!content.contains('\u{1b}'));
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), area);
    assert!(matches!(
        app.modal,
        Some(Modal::Text {
            vertical_scroll: 1,
            ..
        })
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

#[test]
fn ordinary_table_has_aligned_xpdig_style_columns_without_resource() {
    let app = app();
    let snapshot = app.snapshot.as_ref().unwrap();
    let visible = app.visible();
    let plan = TablePlan::new(snapshot, &visible, 140, false, false, false);
    let header = plan.header();
    assert!(header.contains("OBJECT"));
    assert!(header.contains("GROUP"));
    assert!(header.contains("SYNCED LAST"));
    assert!(header.contains("READY LAST"));
    assert!(header.contains("STATUS"));
    assert!(!header.contains("RESOURCE"));
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
fn tree_horizontal_scroll_is_always_available_and_z_does_not_reset_it() {
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
    let offset = app.resource_horizontal_scroll;
    let status = app.status.clone();
    app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE), area);
    assert_eq!(app.resource_horizontal_scroll, offset);
    assert_eq!(app.status, status);
    assert_eq!(rendered_tree(&app, tree).unwrap().lines, rendered.lines);
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
fn tree_horizontal_scrollbar_uses_only_existing_bottom_border() {
    let mut app = app();
    let mut terminal = Terminal::new(TestBackend::new(50, 16)).unwrap();
    let tree = resource_tree_area(Rect::new(0, 0, 50, 16)).unwrap();
    let layout = rendered_tree(&app, tree).unwrap().horizontal;
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let bar_x = tree.x + layout.pinned_width as u16;
    let bar_y = tree.bottom() - 1;
    assert_eq!(
        terminal
            .backend()
            .buffer()
            .cell((bar_x + 1, bar_y))
            .unwrap()
            .symbol(),
        "▪"
    );
    assert_ne!(
        terminal
            .backend()
            .buffer()
            .cell((bar_x - 1, bar_y))
            .unwrap()
            .symbol(),
        "▪"
    );
    app.resource_horizontal_scroll = layout.max_offset();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    assert_eq!(
        terminal
            .backend()
            .buffer()
            .cell((bar_x + layout.viewport_width as u16 - 1, bar_y))
            .unwrap()
            .symbol(),
        "▪"
    );
    let mut wide_terminal = Terminal::new(TestBackend::new(140, 16)).unwrap();
    wide_terminal.draw(|frame| render(frame, &app)).unwrap();
    assert!(!wide_terminal.backend().to_string().contains('▪'));
    app.config.ui.ascii = true;
    terminal.draw(|frame| render(frame, &app)).unwrap();
    assert_eq!(
        terminal
            .backend()
            .buffer()
            .cell((bar_x + layout.viewport_width as u16 - 1, bar_y))
            .unwrap()
            .symbol(),
        "-"
    );
    wide_terminal.draw(|frame| render(frame, &app)).unwrap();
    let wide_tree = resource_tree_area(Rect::new(0, 0, 140, 16)).unwrap();
    assert_ne!(
        wide_terminal
            .backend()
            .buffer()
            .cell((wide_tree.right() - 2, wide_tree.bottom() - 1))
            .unwrap()
            .symbol(),
        ">"
    );
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
                assert_eq!(
                    plan.header().width() + 1,
                    layout.pinned_width + layout.content_width + layout.right_pinned_width
                );
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
    assert!(!rendered.contains("Enter:details"));
    assert!(rendered.contains("Left/Right:collapse/expand"));
    assert!(!rendered.contains("Enter/Space"));
    assert!(rendered.contains("ctrl-d:delete"));
    assert!(!rendered.contains("h/l:scroll"));
    assert!(!rendered.contains("z:fit"));
    assert!(!rendered.contains("z:width"));
    assert!(!HELP_LINES.iter().any(|line| line.starts_with("  z ")));
    assert!(rendered.contains("::command"));
    assert!(!rendered.contains("j/k:move"));
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
                wrapped,
                vertical_scroll: 0,
                horizontal_scroll: 0,
                query: String::new(),
                search_input: None,
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
                app.modal,
                Some(Modal::Text {
                    vertical_scroll: 0,
                    ..
                })
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
                app.modal,
                Some(Modal::Text {
                    vertical_scroll: 3,
                    selection: None,
                    ..
                })
            ));
            app.handle_mouse(
                MouseEvent {
                    modifiers: KeyModifiers::SHIFT,
                    ..event
                },
                area,
            );
            assert!(matches!(
                app.modal,
                Some(Modal::Text {
                    vertical_scroll: 3,
                    ..
                })
            ));
            for _ in 0..100 {
                app.handle_mouse(event, area);
            }
            assert!(
                matches!(app.modal, Some(Modal::Text { vertical_scroll, .. }) if vertical_scroll == max_scroll)
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
                app.modal,
                Some(Modal::Text {
                    vertical_scroll: 0,
                    horizontal_scroll: 0,
                    ..
                })
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
            if let (
                Some(Modal::Text {
                    vertical_scroll: a, ..
                }),
                Some(Modal::Text {
                    vertical_scroll: b, ..
                }),
            ) = (&individual.modal, &batched.modal)
            {
                assert_eq!(a, b);
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
