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

pub(super) fn visual_rows(content: &str, width: u16, wrapped: bool) -> Vec<VisualRow> {
    if width == 0 {
        return Vec::new();
    }
    let mut rows = Vec::new();
    let mut source_offset = 0;
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

#[allow(clippy::too_many_arguments)]
pub(super) fn selection_point_at(
    content: &str,
    body: Rect,
    column: u16,
    row: u16,
    vertical_scroll: u16,
    horizontal_scroll: u16,
    wrapped: bool,
    clamp: bool,
) -> Option<SelectionPoint> {
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
    let visual_row = usize::from(vertical_scroll) + usize::from(row - body.y);
    let rows = visual_rows(content, body.width, wrapped);
    let visual_row = visual_row.min(rows.len().saturating_sub(1));
    let row = rows.get(visual_row)?;
    let target_column = horizontal_scroll.saturating_add(column - body.x);
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

#[allow(clippy::too_many_arguments)]
pub(super) fn render_text_selection(
    frame: &mut ratatui::Frame<'_>,
    body: Rect,
    content: &str,
    selection: TextSelection,
    vertical_scroll: u16,
    horizontal_scroll: u16,
    wrapped: bool,
    theme: &Theme,
) {
    let range = selection.range();
    let rows = visual_rows(content, body.width, wrapped);
    for (screen_row, row) in rows
        .iter()
        .skip(usize::from(vertical_scroll))
        .take(usize::from(body.height))
        .enumerate()
    {
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
                let cell =
                    &mut frame.buffer_mut()[(body.x + screen_column, body.y + screen_row as u16)];
                cell.set_style(theme.text_selection(cell.style()));
            }
        }
    }
}
