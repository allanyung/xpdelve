use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Vertical navigation shared by every list, viewer, and dialog.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Nav {
    Line(isize),
    Page(isize),
    Top,
    Bottom,
}

/// Maps a key to a navigation step. Callers handle their own keys first and
/// must not use this in text-input modes, where `j`/`k` are literal characters.
pub(super) fn nav_key(key: KeyEvent) -> Option<Nav> {
    // Ctrl+J/K come before the plain j/k arms, which accept any modifiers.
    match (key.code, key.modifiers) {
        (KeyCode::PageDown, _) | (KeyCode::Char('j'), KeyModifiers::CONTROL) => Some(Nav::Page(1)),
        (KeyCode::PageUp, _) | (KeyCode::Char('k'), KeyModifiers::CONTROL) => Some(Nav::Page(-1)),
        (KeyCode::Down | KeyCode::Char('j'), _) => Some(Nav::Line(1)),
        (KeyCode::Up | KeyCode::Char('k'), _) => Some(Nav::Line(-1)),
        (KeyCode::Home | KeyCode::Char('g'), _) => Some(Nav::Top),
        (KeyCode::End | KeyCode::Char('G'), _) => Some(Nav::Bottom),
        _ => None,
    }
}

/// Applies a step to a clamped offset in `0..=max`.
pub(super) fn apply_scroll(offset: u16, nav: Nav, page: usize, max: u16) -> u16 {
    let page = u16::try_from(page).unwrap_or(u16::MAX);
    match nav {
        Nav::Line(direction) if direction < 0 => offset.saturating_sub(1),
        Nav::Line(_) => offset.saturating_add(1).min(max),
        Nav::Page(direction) if direction < 0 => offset.saturating_sub(page),
        Nav::Page(_) => offset.saturating_add(page).min(max),
        Nav::Top => 0,
        Nav::Bottom => max,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Option<Nav> {
        nav_key(KeyEvent::new(code, modifiers))
    }

    #[test]
    fn maps_navigation_keys() {
        let none = KeyModifiers::NONE;
        let ctrl = KeyModifiers::CONTROL;
        assert_eq!(key(KeyCode::Char('j'), none), Some(Nav::Line(1)));
        assert_eq!(key(KeyCode::Down, none), Some(Nav::Line(1)));
        assert_eq!(key(KeyCode::Char('k'), none), Some(Nav::Line(-1)));
        assert_eq!(key(KeyCode::Up, none), Some(Nav::Line(-1)));
        assert_eq!(key(KeyCode::PageDown, none), Some(Nav::Page(1)));
        assert_eq!(key(KeyCode::Char('j'), ctrl), Some(Nav::Page(1)));
        assert_eq!(key(KeyCode::PageUp, none), Some(Nav::Page(-1)));
        assert_eq!(key(KeyCode::Char('k'), ctrl), Some(Nav::Page(-1)));
        assert_eq!(key(KeyCode::Home, none), Some(Nav::Top));
        assert_eq!(key(KeyCode::Char('g'), none), Some(Nav::Top));
        assert_eq!(key(KeyCode::End, none), Some(Nav::Bottom));
        assert_eq!(
            key(KeyCode::Char('G'), KeyModifiers::SHIFT),
            Some(Nav::Bottom)
        );
        assert_eq!(key(KeyCode::Char('x'), none), None);
    }

    #[test]
    fn scroll_clamps() {
        assert_eq!(apply_scroll(0, Nav::Line(-1), 10, 50), 0);
        assert_eq!(apply_scroll(48, Nav::Page(1), 10, 50), 50);
        assert_eq!(apply_scroll(20, Nav::Page(-1), 10, 50), 10);
        assert_eq!(apply_scroll(5, Nav::Bottom, 10, 50), 50);
        assert_eq!(apply_scroll(5, Nav::Top, 10, 50), 0);
    }
}
