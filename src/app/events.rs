use super::*;

pub(super) const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const SCROLL_QUIET_PERIOD: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug)]
pub(super) struct WheelBatch {
    pub(super) mouse: MouseEvent,
    pub(super) count: usize,
}

// Keep only one accumulator, not an application queue of thousands of wheel
// events. Keys, clicks, resize events, and wheel direction/position changes are
// ordering boundaries and are never swallowed by coalescing.
#[derive(Default)]
pub(super) struct WheelInput {
    pending: Option<WheelBatch>,
    last_wheel: Option<Instant>,
    suppress_until: Option<Instant>,
}

impl WheelInput {
    pub(super) fn push(
        &mut self,
        event: TerminalEvent,
        now: Instant,
    ) -> (Option<WheelBatch>, Option<TerminalEvent>) {
        if let TerminalEvent::Mouse(mouse) = event
            && matches!(
                mouse.kind,
                MouseEventKind::ScrollUp
                    | MouseEventKind::ScrollDown
                    | MouseEventKind::ScrollLeft
                    | MouseEventKind::ScrollRight
            )
        {
            self.last_wheel = Some(now);
            if self.suppress_until.is_some_and(|until| now < until) {
                // Also suppress trackpad momentum that arrives after Esc, until
                // the old gesture has stopped. Ordinary input remains usable.
                self.suppress_until = Some(now + SCROLL_QUIET_PERIOD);
                return (None, None);
            }
            self.suppress_until = None;
            if let Some(pending) = &mut self.pending
                && pending.mouse == mouse
            {
                pending.count = pending.count.saturating_add(1);
                return (None, None);
            }
            let previous = self.pending.replace(WheelBatch { mouse, count: 1 });
            return (previous, None);
        }
        (self.take(), Some(event))
    }

    pub(super) fn take(&mut self) -> Option<WheelBatch> {
        self.pending.take()
    }

    pub(super) fn view_changed(&mut self, now: Instant) {
        self.pending = None;
        if self
            .last_wheel
            .is_some_and(|last| now.duration_since(last) < SCROLL_QUIET_PERIOD)
        {
            self.suppress_until = Some(now + SCROLL_QUIET_PERIOD);
        }
    }

    pub(super) fn exclude_draw_time(&mut self, started: Instant, finished: Instant) {
        // A blocked renderer is not evidence that a gesture has stopped: wheel
        // events may still be queued in the terminal while we cannot read them.
        let elapsed = finished.duration_since(started);
        if let Some(until) = &mut self.suppress_until
            && *until > started
        {
            *until += elapsed;
        }
        if let Some(last) = &mut self.last_wheel
            && started.duration_since(*last) < SCROLL_QUIET_PERIOD
        {
            *last += elapsed;
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct InputView {
    mode: InputMode,
    modal: Option<std::mem::Discriminant<Modal>>,
    document: Option<u64>,
}

impl InputView {
    pub(super) fn of(app: &App) -> Self {
        Self {
            mode: app.mode,
            modal: app.modal.as_ref().map(std::mem::discriminant),
            document: match &app.modal {
                Some(Modal::Text { content, .. }) => Some(content.id()),
                _ => None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wheel(kind: MouseEventKind) -> TerminalEvent {
        TerminalEvent::Mouse(MouseEvent {
            kind,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        })
    }

    #[test]
    fn large_wheel_burst_is_one_update_before_the_next_key() {
        let mut input = WheelInput::default();
        let now = Instant::now();
        for _ in 0..50_000 {
            let (batch, event) = input.push(wheel(MouseEventKind::ScrollDown), now);
            assert!(batch.is_none());
            assert!(event.is_none());
        }
        let escape = TerminalEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let (batch, event) = input.push(escape.clone(), now);
        assert_eq!(batch.unwrap().count, 50_000);
        assert_eq!(event, Some(escape));
        assert!(input.take().is_none());
    }

    #[test]
    fn wheel_batches_preserve_direction_modifiers_and_pointer_boundaries() {
        let now = Instant::now();
        let mut input = WheelInput::default();
        let mut mouse = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        input.push(TerminalEvent::Mouse(mouse), now);
        for next in [
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                ..mouse
            },
            MouseEvent {
                modifiers: KeyModifiers::SHIFT,
                ..mouse
            },
            MouseEvent { column: 6, ..mouse },
            MouseEvent { row: 6, ..mouse },
            MouseEvent {
                kind: MouseEventKind::ScrollRight,
                ..mouse
            },
        ] {
            let (batch, event) = input.push(TerminalEvent::Mouse(next), now);
            let batch = batch.unwrap();
            assert_eq!(batch.mouse, mouse);
            assert_eq!(batch.count, 1);
            assert!(event.is_none());
            mouse = next;
        }
        assert_eq!(input.take().unwrap().mouse, mouse);
    }

    #[test]
    fn wheel_batches_never_swallow_keys_clicks_drags_or_resizes() {
        let now = Instant::now();
        let mut input = WheelInput::default();
        for boundary in [
            TerminalEvent::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
            wheel(MouseEventKind::Down(MouseButton::Left)),
            wheel(MouseEventKind::Drag(MouseButton::Left)),
            wheel(MouseEventKind::Up(MouseButton::Left)),
            TerminalEvent::Resize(100, 30),
            TerminalEvent::Paste("hello".into()),
        ] {
            input.push(wheel(MouseEventKind::ScrollDown), now);
            let (batch, event) = input.push(boundary.clone(), now);
            assert_eq!(batch.unwrap().count, 1);
            assert_eq!(event, Some(boundary));
        }
    }

    #[test]
    fn view_transition_discards_wheel_tail_and_momentum_but_not_other_input() {
        let now = Instant::now();
        let mut input = WheelInput::default();
        input.push(wheel(MouseEventKind::ScrollDown), now);
        input.view_changed(now);
        assert!(input.take().is_none());
        for index in 1..=20 {
            let (batch, event) = input.push(
                wheel(MouseEventKind::ScrollDown),
                now + Duration::from_millis(index * 20),
            );
            assert!(batch.is_none());
            assert!(event.is_none());
        }
        let quit = TerminalEvent::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        let (batch, event) = input.push(quit.clone(), now + Duration::from_millis(401));
        assert!(batch.is_none());
        assert_eq!(event, Some(quit));
        let click = wheel(MouseEventKind::Down(MouseButton::Left));
        let (_, event) = input.push(click.clone(), now + Duration::from_millis(402));
        assert_eq!(event, Some(click));
        input.push(
            wheel(MouseEventKind::ScrollDown),
            now + Duration::from_millis(400) + SCROLL_QUIET_PERIOD,
        );
        assert_eq!(input.take().unwrap().count, 1);
    }

    #[test]
    fn slow_redraw_does_not_expire_the_view_transition_guard() {
        let now = Instant::now();
        let mut input = WheelInput::default();
        input.push(wheel(MouseEventKind::ScrollDown), now);
        let finished = now + Duration::from_millis(300);
        input.exclude_draw_time(now, finished);
        input.view_changed(finished);
        input.exclude_draw_time(finished, finished + Duration::from_millis(300));
        let (batch, event) = input.push(
            wheel(MouseEventKind::ScrollDown),
            finished + Duration::from_millis(300),
        );
        assert!(batch.is_none());
        assert!(event.is_none());
        assert!(input.take().is_none());
        input.push(
            wheel(MouseEventKind::ScrollDown),
            finished + Duration::from_millis(300) + SCROLL_QUIET_PERIOD,
        );
        assert!(input.take().is_some());
    }

    #[test]
    fn view_transition_without_recent_scrolling_allows_a_new_gesture() {
        let now = Instant::now();
        let mut input = WheelInput::default();
        input.view_changed(now);
        input.push(wheel(MouseEventKind::ScrollDown), now);
        assert!(input.take().is_some());
        input.view_changed(now + SCROLL_QUIET_PERIOD);
        input.push(wheel(MouseEventKind::ScrollUp), now + SCROLL_QUIET_PERIOD);
        assert!(input.take().is_some());
    }
}
