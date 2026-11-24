use super::*;

#[derive(Clone, Debug)]
pub(super) struct VisualGrapheme {
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) width: u16,
    pub(super) whitespace: bool,
}

#[derive(Clone, Debug)]
pub(super) struct VisualRow {
    pub(super) graphemes: Vec<VisualGrapheme>,
    pub(super) source_start: usize,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Viewport {
    pub(super) vertical: u16,
    pub(super) horizontal: u16,
    pub(super) wrapped: bool,
}

pub(super) const TREE_VIEWPORT: Viewport = Viewport {
    vertical: 0,
    horizontal: 0,
    wrapped: false,
};

pub(super) fn visual_rows(content: &str, width: u16, wrapped: bool) -> Vec<VisualRow> {
    visual_rows_from(content, 0, width, wrapped)
}

/// Rows of `content`, whose first byte sits at `base` in the full document.
pub(super) fn visual_rows_from(
    content: &str,
    base: usize,
    width: u16,
    wrapped: bool,
) -> Vec<VisualRow> {
    if width == 0 {
        return Vec::new();
    }
    let mut rows = Vec::new();
    let mut source_offset = base;
    for raw_line in content.split_inclusive('\n') {
        let without_newline = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        let line = without_newline
            .strip_suffix('\r')
            .unwrap_or(without_newline);
        let graphemes = UnicodeSegmentation::grapheme_indices(line, true)
            .map(|(offset, symbol)| VisualGrapheme {
                start: source_offset + offset,
                end: source_offset + offset + symbol.len(),
                width: symbol.width().try_into().unwrap_or(u16::MAX),
                whitespace: symbol.chars().all(char::is_whitespace),
            })
            .collect::<Vec<_>>();
        if wrapped {
            rows.extend(wrap_visual_line(graphemes, source_offset, width));
        } else {
            rows.push(VisualRow {
                graphemes,
                source_start: source_offset,
            });
        }
        source_offset += raw_line.len();
    }
    rows
}

// Mirrors Ratatui's WordWrapper with trim=false, while retaining source offsets
// so mouse coordinates can be translated back into the original YAML.
pub(super) fn wrap_visual_line(
    graphemes: Vec<VisualGrapheme>,
    source_start: usize,
    max_width: u16,
) -> Vec<VisualRow> {
    let mut rows = Vec::new();
    let mut pending_line = Vec::new();
    let mut pending_word = Vec::new();
    let mut pending_whitespace = VecDeque::new();
    let mut line_width = 0_u16;
    let mut word_width = 0_u16;
    let mut whitespace_width = 0_u16;
    let mut non_whitespace_previous = false;

    for grapheme in graphemes {
        if grapheme.width > max_width {
            continue;
        }
        let is_whitespace = grapheme.whitespace;
        let word_found = non_whitespace_previous && is_whitespace;
        let untrimmed_overflow = pending_line.is_empty()
            && word_width
                .saturating_add(whitespace_width)
                .saturating_add(grapheme.width)
                > max_width;
        if word_found || untrimmed_overflow {
            pending_line.extend(pending_whitespace.drain(..));
            line_width = line_width.saturating_add(whitespace_width);
            pending_line.append(&mut pending_word);
            line_width = line_width.saturating_add(word_width);
            whitespace_width = 0;
            word_width = 0;
        }

        let line_full = line_width >= max_width;
        let pending_word_overflow = grapheme.width > 0
            && line_width
                .saturating_add(whitespace_width)
                .saturating_add(word_width)
                >= max_width;
        if line_full || pending_word_overflow {
            let mut remaining = max_width.saturating_sub(line_width);
            rows.push(VisualRow {
                source_start: pending_line
                    .first()
                    .map_or(source_start, |item: &VisualGrapheme| item.start),
                graphemes: std::mem::take(&mut pending_line),
            });
            line_width = 0;
            while let Some(item) = pending_whitespace.front() {
                if item.width > remaining {
                    break;
                }
                whitespace_width = whitespace_width.saturating_sub(item.width);
                remaining = remaining.saturating_sub(item.width);
                pending_whitespace.pop_front();
            }
            if is_whitespace && pending_whitespace.is_empty() {
                continue;
            }
        }

        if is_whitespace {
            whitespace_width = whitespace_width.saturating_add(grapheme.width);
            pending_whitespace.push_back(grapheme);
        } else {
            word_width = word_width.saturating_add(grapheme.width);
            pending_word.push(grapheme);
        }
        non_whitespace_previous = !is_whitespace;
    }

    pending_line.extend(pending_whitespace);
    pending_line.append(&mut pending_word);
    if !pending_line.is_empty() {
        rows.push(VisualRow {
            source_start: pending_line[0].start,
            graphemes: pending_line,
        });
    }
    if rows.is_empty() {
        rows.push(VisualRow {
            graphemes: Vec::new(),
            source_start,
        });
    }
    rows
}

/// Clamps a pointer position into `body`, returning its offset from the body origin.
pub(super) fn body_offset(body: Rect, column: u16, row: u16, clamp: bool) -> Option<(u16, u16)> {
    if body.is_empty() {
        return None;
    }
    let inside = column >= body.x
        && column < body.x.saturating_add(body.width)
        && row >= body.y
        && row < body.y.saturating_add(body.height);
    if !inside && !clamp {
        return None;
    }
    let column = column.clamp(body.x, body.x.saturating_add(body.width).saturating_sub(1));
    let row = row.clamp(body.y, body.y.saturating_add(body.height).saturating_sub(1));
    Some((column - body.x, row - body.y))
}

pub(super) fn point_in_row(
    row: &VisualRow,
    target_column: u16,
    clamp: bool,
) -> Option<SelectionPoint> {
    let mut current_column = 0_u16;
    for grapheme in &row.graphemes {
        let end_column = current_column.saturating_add(grapheme.width.max(1));
        if target_column < end_column {
            return Some(SelectionPoint {
                start: grapheme.start,
                end: grapheme.end,
            });
        }
        current_column = end_column;
    }
    if clamp {
        return row.graphemes.last().map_or(
            Some(SelectionPoint {
                start: row.source_start,
                end: row.source_start,
            }),
            |grapheme| {
                Some(SelectionPoint {
                    start: grapheme.start,
                    end: grapheme.end,
                })
            },
        );
    }
    None
}

pub(super) fn paint_row(
    frame: &mut ratatui::Frame<'_>,
    body: Rect,
    screen_row: u16,
    row: &VisualRow,
    range: &std::ops::Range<usize>,
    horizontal_scroll: u16,
    theme: &Theme,
) {
    let mut visual_column = 0_u16;
    for grapheme in &row.graphemes {
        let grapheme_column = visual_column;
        visual_column = visual_column.saturating_add(grapheme.width.max(1));
        if grapheme.start >= range.end || grapheme.end <= range.start {
            continue;
        }
        for offset in 0..grapheme.width.max(1) {
            let column = grapheme_column.saturating_add(offset);
            if column < horizontal_scroll {
                continue;
            }
            let screen_column = column - horizontal_scroll;
            if screen_column >= body.width {
                continue;
            }
            let cell = &mut frame.buffer_mut()[(body.x + screen_column, body.y + screen_row)];
            cell.set_style(theme.text_selection(cell.style()));
        }
    }
}
