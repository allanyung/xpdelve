use std::cell::{OnceCell, RefCell};
use std::ops::{Deref, Range};
use std::sync::atomic::{AtomicU64, Ordering};

use super::render::styled_content_line;
use super::selection::visual_rows;
use super::*;

static NEXT_DOCUMENT_ID: AtomicU64 = AtomicU64::new(0);

// Text is immutable for the lifetime of a view, so scrolling never invalidates
// its layout or syntax cache. Opening another document creates a fresh cache.
#[derive(Clone, Debug)]
pub(super) struct TextContent {
    id: u64,
    text: String,
    line_ranges: OnceCell<Vec<Range<usize>>>,
    cache: RefCell<TextCache>,
}

#[derive(Clone, Debug, Default)]
struct TextCache {
    layout: Option<TextLayout>,
    styling: Option<TextStyling>,
}

#[derive(Clone, Debug)]
struct TextLayout {
    width: u16,
    wrapped: bool,
    starts: Vec<usize>,
    rows: usize,
    max_line_width: usize,
}

#[derive(Clone, Debug)]
struct TextStyling {
    kind: ContentKind,
    query: String,
    palette: crate::theme::Palette,
    colors_enabled: bool,
    lines: Vec<Option<Line<'static>>>,
}

impl TextCache {
    fn layout(&mut self, text: &str, width: u16, wrapped: bool) -> &TextLayout {
        if self
            .layout
            .as_ref()
            .is_none_or(|layout| layout.width != width || layout.wrapped != wrapped)
        {
            let mut starts = Vec::new();
            let mut rows = 0;
            let mut max_line_width = 0;
            for line in text.lines() {
                starts.push(rows);
                max_line_width = max_line_width.max(line.width());
                rows += if wrapped {
                    visual_rows(line, width, true).len().max(1)
                } else {
                    1
                };
            }
            self.layout = Some(TextLayout {
                width,
                wrapped,
                starts,
                rows,
                max_line_width,
            });
        }
        self.layout.as_ref().unwrap()
    }

    fn style(&mut self, line_count: usize, kind: ContentKind, query: &str, theme: &Theme) {
        if self.styling.as_ref().is_none_or(|styling| {
            styling.kind != kind
                || styling.query != query
                || styling.palette != theme.palette
                || styling.colors_enabled != theme.colors_enabled
        }) {
            self.styling = Some(TextStyling {
                kind,
                query: query.into(),
                palette: theme.palette,
                colors_enabled: theme.colors_enabled,
                lines: vec![None; line_count],
            });
        }
    }
}

impl TextContent {
    pub(super) fn id(&self) -> u64 {
        self.id
    }

    fn line_ranges(&self) -> &[Range<usize>] {
        self.line_ranges.get_or_init(|| {
            self.text
                .lines()
                .map(|line| {
                    let start = line.as_ptr() as usize - self.text.as_ptr() as usize;
                    start..start + line.len()
                })
                .collect()
        })
    }

    pub(super) fn scroll_bounds(&self, width: u16, height: u16, wrapped: bool) -> (u16, u16) {
        let mut cache = self.cache.borrow_mut();
        let layout = cache.layout(&self.text, width, wrapped);
        let vertical = layout.rows.saturating_sub(usize::from(height));
        let horizontal = if wrapped {
            0
        } else {
            layout.max_line_width.saturating_sub(usize::from(width))
        };
        (
            vertical.try_into().unwrap_or(u16::MAX),
            horizontal.try_into().unwrap_or(u16::MAX),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render(
        &self,
        frame: &mut ratatui::Frame<'_>,
        body: Rect,
        kind: ContentKind,
        query: &str,
        wrapped: bool,
        vertical_scroll: u16,
        horizontal_scroll: u16,
        theme: &Theme,
    ) {
        let mut cache = self.cache.borrow_mut();
        let layout = cache.layout(&self.text, body.width, wrapped);
        let scroll = usize::from(vertical_scroll);
        let first = layout
            .starts
            .partition_point(|start| *start <= scroll)
            .saturating_sub(1);
        let last = layout
            .starts
            .partition_point(|start| *start < scroll + usize::from(body.height));
        let offset = layout
            .starts
            .get(first)
            .map_or(0, |start| scroll.saturating_sub(*start));
        let ranges = self.line_ranges();
        cache.style(ranges.len(), kind, query, theme);
        // Borrow only the visible logical lines; neither restyling nor cloning
        // the entire document is necessary to draw another scroll position.
        let styling = cache.styling.as_mut().unwrap();
        for (index, range) in ranges.iter().enumerate().take(last).skip(first.min(last)) {
            styling.lines[index].get_or_insert_with(|| {
                let styled = styled_content_line(&self.text[range.clone()], kind, query, theme);
                Line::from(
                    styled
                        .spans
                        .into_iter()
                        .map(|span| Span::styled(span.content.into_owned(), span.style))
                        .collect::<Vec<_>>(),
                )
                .style(styled.style)
            });
        }
        let lines = styling.lines[first.min(last)..last]
            .iter()
            .map(|line| {
                let line = line.as_ref().unwrap();
                Line::from(
                    line.spans
                        .iter()
                        .map(|span| Span::styled(span.content.as_ref(), span.style))
                        .collect::<Vec<_>>(),
                )
                .style(line.style)
            })
            .collect::<Vec<_>>();
        let mut paragraph = Paragraph::new(lines)
            .scroll((offset.try_into().unwrap_or(u16::MAX), horizontal_scroll));
        if wrapped {
            paragraph = paragraph.wrap(Wrap { trim: false });
        }
        frame.render_widget(paragraph, body);
    }
}

impl Deref for TextContent {
    type Target = str;

    fn deref(&self) -> &str {
        &self.text
    }
}

impl From<String> for TextContent {
    fn from(text: String) -> Self {
        Self {
            id: NEXT_DOCUMENT_ID.fetch_add(1, Ordering::Relaxed),
            text,
            line_ranges: OnceCell::default(),
            cache: RefCell::default(),
        }
    }
}

impl From<&str> for TextContent {
    fn from(text: &str) -> Self {
        text.to_owned().into()
    }
}

impl PartialEq for TextContent {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl PartialEq<str> for TextContent {
    fn eq(&self, other: &str) -> bool {
        self.text == other
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn theme() -> Theme {
        Theme::resolve_for_mode(
            &Config::default().skin,
            crate::config::ColorMode::Always,
            false,
            Some(terminal_colorsaurus::ThemeMode::Dark),
        )
        .unwrap()
    }

    #[test]
    fn cached_viewport_matches_full_paragraph_at_every_scroll_position() {
        let content = TextContent::from(concat!(
            "# comment\r\n",
            "kind: Example\r\n",
            "\r\n",
            "value: one word another word with trailing spaces   \n",
            "unicode: 界界 e\u{301} 👩‍💻 👨‍👩‍👧‍👦\n",
            "   deeply: indented value with words\n",
            "long: abcdefghijklmnopqrstuvwxyz0123456789\n",
            "   \n",
            "flag: true\n",
        ));
        let theme = theme();
        for width in [8, 12, 32] {
            for wrapped in [false, true] {
                for query in ["", "word", "true"] {
                    let (max_scroll, _) = content.scroll_bounds(width, 3, wrapped);
                    let mut cached = Terminal::new(TestBackend::new(width, 3)).unwrap();
                    let mut reference = Terminal::new(TestBackend::new(width, 3)).unwrap();
                    for scroll in 0..=max_scroll + 3 {
                        let horizontal = if wrapped { 0 } else { 3 };
                        cached
                            .draw(|frame| {
                                content.render(
                                    frame,
                                    frame.area(),
                                    ContentKind::Yaml,
                                    query,
                                    wrapped,
                                    scroll,
                                    horizontal,
                                    &theme,
                                );
                            })
                            .unwrap();
                        reference
                            .draw(|frame| {
                                let lines = content
                                    .lines()
                                    .map(|line| {
                                        styled_content_line(line, ContentKind::Yaml, query, &theme)
                                    })
                                    .collect::<Vec<_>>();
                                let mut paragraph =
                                    Paragraph::new(lines).scroll((scroll, horizontal));
                                if wrapped {
                                    paragraph = paragraph.wrap(Wrap { trim: false });
                                }
                                frame.render_widget(paragraph, frame.area());
                            })
                            .unwrap();
                        assert_eq!(
                            cached.backend().buffer(),
                            reference.backend().buffer(),
                            "width={width} wrapped={wrapped} query={query} scroll={scroll}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn scrolling_reuses_layout_and_styles_only_visible_lines() {
        let content = TextContent::from("key: a value\n".repeat(1_000));
        let theme = theme();
        let mut terminal = Terminal::new(TestBackend::new(30, 4)).unwrap();
        terminal
            .draw(|frame| {
                content.render(
                    frame,
                    frame.area(),
                    ContentKind::Yaml,
                    "",
                    true,
                    0,
                    0,
                    &theme,
                );
            })
            .unwrap();
        let layout_pointer;
        let styled_pointer;
        {
            let cache = content.cache.borrow();
            layout_pointer = cache.layout.as_ref().unwrap().starts.as_ptr();
            let styling = cache.styling.as_ref().unwrap();
            assert_eq!(
                styling.lines.iter().filter(|line| line.is_some()).count(),
                4
            );
            styled_pointer = styling.lines[0].as_ref().unwrap().spans[0].content.as_ptr();
        }
        for height in 1..=10 {
            content.scroll_bounds(30, height, true);
        }
        terminal
            .draw(|frame| {
                content.render(
                    frame,
                    frame.area(),
                    ContentKind::Yaml,
                    "",
                    true,
                    1,
                    0,
                    &theme,
                );
            })
            .unwrap();
        {
            let cache = content.cache.borrow();
            assert_eq!(
                cache.layout.as_ref().unwrap().starts.as_ptr(),
                layout_pointer
            );
            let styling = cache.styling.as_ref().unwrap();
            assert_eq!(
                styling.lines.iter().filter(|line| line.is_some()).count(),
                5
            );
            assert_eq!(
                styling.lines[0].as_ref().unwrap().spans[0].content.as_ptr(),
                styled_pointer
            );
        }
        terminal
            .draw(|frame| {
                content.render(
                    frame,
                    frame.area(),
                    ContentKind::Yaml,
                    "value",
                    true,
                    1,
                    0,
                    &theme,
                );
            })
            .unwrap();
        let cache = content.cache.borrow();
        assert_eq!(
            cache.layout.as_ref().unwrap().starts.as_ptr(),
            layout_pointer
        );
        let styling = cache.styling.as_ref().unwrap();
        assert!(styling.lines[0].is_none());
        assert_eq!(
            styling.lines.iter().filter(|line| line.is_some()).count(),
            4
        );
    }

    #[test]
    fn cached_styles_invalidate_for_theme_color_mode_and_content_kind() {
        use ratatui::style::Color;

        let content = TextContent::from("key: value");
        let mut theme = theme();
        let mut terminal = Terminal::new(TestBackend::new(30, 4)).unwrap();
        for (kind, colors_enabled, sky) in [
            (ContentKind::Yaml, true, Color::Red),
            (ContentKind::Yaml, true, Color::Green),
            (ContentKind::Yaml, false, Color::Green),
            (ContentKind::Describe, true, Color::Green),
        ] {
            theme.colors_enabled = colors_enabled;
            theme.palette.sky = sky;
            terminal
                .draw(|frame| {
                    content.render(frame, frame.area(), kind, "", true, 0, 0, &theme);
                })
                .unwrap();
            let expected = if colors_enabled && kind == ContentKind::Yaml {
                sky
            } else {
                Color::Reset
            };
            assert_eq!(
                terminal.backend().buffer().cell((0, 0)).unwrap().fg,
                expected
            );
        }
    }

    #[test]
    fn cached_bounds_invalidate_for_width_and_wrap_changes() {
        let content = TextContent::from("words words words words words\n\nend\n");
        let narrow = content.scroll_bounds(8, 2, true).0;
        let wide = content.scroll_bounds(40, 2, true).0;
        assert!(narrow > wide);
        assert_eq!(content.scroll_bounds(8, 2, false), (1, 21));
        assert_eq!(content.scroll_bounds(8, 2, true).0, narrow);
    }
}
