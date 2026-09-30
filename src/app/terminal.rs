use super::*;

pub(super) struct TerminalGuard {
    pub(super) terminal: Terminal<CrosstermBackend<io::Stdout>>,
    mouse_capture: bool,
}

impl TerminalGuard {
    pub(super) fn enter() -> Result<Self> {
        enable_raw_mode().context("failed to enable terminal raw mode")?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(error).context("failed to enter alternate screen");
        }
        let terminal = match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => terminal,
            Err(error) => {
                let mut stdout = io::stdout();
                let _ = execute!(stdout, DisableMouseCapture, LeaveAlternateScreen);
                let _ = disable_raw_mode();
                return Err(error.into());
            }
        };
        Ok(Self {
            terminal,
            mouse_capture: false,
        })
    }

    pub(super) fn set_mouse_capture(&mut self, enabled: bool) -> Result<()> {
        if enabled == self.mouse_capture {
            return Ok(());
        }
        if enabled {
            execute!(self.terminal.backend_mut(), EnableMouseCapture)?;
        } else {
            execute!(self.terminal.backend_mut(), DisableMouseCapture)?;
        }
        self.mouse_capture = enabled;
        Ok(())
    }

    pub(super) fn suspend(&mut self) -> Result<()> {
        disable_raw_mode()?;
        if self.mouse_capture {
            execute!(self.terminal.backend_mut(), DisableMouseCapture)?;
        }
        execute!(self.terminal.backend_mut(), LeaveAlternateScreen)?;
        self.terminal.show_cursor()?;
        Ok(())
    }

    pub(super) fn resume(&mut self) -> Result<()> {
        enable_raw_mode()?;
        execute!(self.terminal.backend_mut(), EnterAlternateScreen)?;
        if self.mouse_capture {
            execute!(self.terminal.backend_mut(), EnableMouseCapture)?;
        }
        self.terminal.clear()?;
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            DisableMouseCapture,
            LeaveAlternateScreen
        );
        let _ = self.terminal.show_cursor();
    }
}
