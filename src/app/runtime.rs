use super::events::{FRAME_INTERVAL, InputView, WheelBatch, WheelInput};
use super::render::render;
use super::terminal::TerminalGuard;
use super::*;

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
    let mut wheel_input = WheelInput::default();
    let mut input_view = InputView::of(&app);
    let mut next_frame = time::Instant::now();
    let mut dirty = true;
    connect_kubernetes(cli, &sender);
    request_refresh(&mut app, cli, &sender, &mut active, true);

    while !app.quit {
        update_refresh_interval(&mut refresh, app.config.interval());
        app.apply_deferred_snapshot();
        let current_view = InputView::of(&app);
        if current_view != input_view {
            wheel_input.view_changed(Instant::now());
            input_view = current_view;
        }
        terminal.set_mouse_capture(app.captures_mouse())?;
        let toast_active = app.toast.is_some();
        let toast_delay = app
            .toast
            .as_ref()
            .map_or(Duration::from_secs(86_400), |toast| {
                toast.expires_at.saturating_duration_since(Instant::now())
            });
        tokio::select! {
            _ = time::sleep_until(next_frame) => {
                let started = Instant::now();
                if let Some(batch) = wheel_input.take() {
                    apply_wheel_batch(&mut app, &mut terminal, batch)?;
                    dirty = true;
                }
                if dirty {
                    terminal.terminal.draw(|frame| {
                        app.clamp_tree_horizontal_scroll(frame.area());
                        render(frame, &app);
                    })?;
                    dirty = false;
                }
                wheel_input.exclude_draw_time(started, Instant::now());
                // Always leave time to consume input after a slow draw. An
                // overdue repeating timer could otherwise monopolize the loop.
                next_frame = time::Instant::now() + FRAME_INTERVAL;
            }
            event = events.next() => {
                let Some(event) = event else { break };
                let event = event.context("failed to read terminal event")?;
                let (batch, event) = wheel_input.push(event, Instant::now());
                if let Some(batch) = batch {
                    apply_wheel_batch(&mut app, &mut terminal, batch)?;
                    dirty = true;
                }
                let Some(event) = event else { continue };
                dirty = true;
                match event {
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
                        match app.handle_mouse(mouse, area.into()) {
                            Some(MouseAction::Copy(value)) => match copy_osc52(&value) {
                                Ok(()) => app.show_toast("● Copied to clipboard"),
                                Err(error) => app.status = format!("Copy failed: {error}"),
                            },
                            Some(MouseAction::Action(action)) => {
                                if let UiAction::Edit(target) = action {
                                    let success = run_kubectl(&mut terminal, &app, cli, target, "edit").await;
                                    let should_refresh = success.is_ok();
                                    app.status = match success {
                                        Ok(()) => "Edit completed".into(),
                                        Err(error) => format!("Edit failed: {error}"),
                                    };
                                    if should_refresh {
                                        request_refresh(&mut app, cli, &sender, &mut active, false);
                                    }
                                } else {
                                    start_action(&mut app, action, &sender, cli);
                                }
                            }
                            None => {}
                        }
                    }
                    _ => {}
                }
            }
            Some(event) = receiver.recv() => {
                dirty = true;
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
                        let succeeded = app.finish_action(&label, identity, result);
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
                dirty = true;
            }
            _ = time::sleep(toast_delay), if toast_active => {
                app.toast = None;
                dirty = true;
            }
        }
    }
    if let Some(token) = active {
        token.cancel();
        time::sleep(Duration::from_millis(800)).await;
    }
    Ok(())
}

fn apply_wheel_batch(app: &mut App, terminal: &mut TerminalGuard, batch: WheelBatch) -> Result<()> {
    let area = terminal.terminal.size()?;
    let action = app.handle_mouse_events(batch.mouse, batch.count, area.into());
    debug_assert!(
        action.is_none(),
        "wheel events must not dispatch resource actions"
    );
    Ok(())
}

pub(super) fn update_refresh_interval(refresh: &mut time::Interval, interval: Duration) {
    if refresh.period() != interval {
        *refresh = time::interval_at(time::Instant::now() + interval, interval);
        refresh.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    }
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
        let result = result.map_err(|error| format!("{error:#}"));
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

pub(super) fn kubectl_args(
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
