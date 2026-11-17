use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::{
    app::{App, Focus, LayoutMode},
    diff::{Kind, Row},
    git::{View, display_path, display_text},
};

pub const DELETED: Color = Color::Rgb(58, 27, 34);
pub const ADDED: Color = Color::Rgb(22, 51, 39);
pub const DELETED_EMPHASIS: Color = Color::Rgb(133, 45, 60);
pub const ADDED_EMPHASIS: Color = Color::Rgb(35, 104, 65);
const BACKGROUND: Color = Color::Rgb(16, 21, 30);
const FOREGROUND: Color = Color::Rgb(218, 226, 237);
const MUTED: Color = Color::Rgb(130, 148, 170);
const ACCENT: Color = Color::Rgb(113, 192, 227);

pub fn draw(frame: &mut Frame, app: &mut App, busy: bool) {
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(BACKGROUND).fg(FOREGROUND)),
        area,
    );
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .areas(area);
    let title = Line::from(vec![
        Span::styled(" diffio ", Style::default().fg(ACCENT).bold()),
        Span::styled(display_path(&app.root), Style::default().fg(MUTED)),
        Span::raw(if busy {
            "  • refreshing"
        } else {
            "  • live / 500 ms"
        }),
    ]);
    frame.render_widget(
        Paragraph::new(title),
        Rect {
            height: header.height.min(1),
            ..header
        },
    );
    let tabs: Vec<_> = [View::All, View::Staged, View::Unstaged]
        .iter()
        .enumerate()
        .flat_map(|(i, view)| {
            let style = if app.view == *view {
                Style::default().fg(BACKGROUND).bg(ACCENT).bold()
            } else {
                Style::default().fg(MUTED)
            };
            [
                Span::styled(format!(" {} {} ", i + 1, view.label()), style),
                Span::raw(" "),
            ]
        })
        .collect();
    frame.render_widget(
        Paragraph::new(Line::from(tabs)),
        Rect {
            y: header.y.saturating_add(1),
            height: header.height.saturating_sub(1),
            ..header
        },
    );

    let (tree, diff) = if app.show_tree && body.width >= 60 {
        let [tree, diff] = Layout::horizontal([
            Constraint::Length((body.width / 4).clamp(22, 38)),
            Constraint::Min(0),
        ])
        .areas(body);
        (tree, diff)
    } else if app.show_tree && app.focus == Focus::Tree {
        (body, Rect::default())
    } else {
        (Rect::default(), body)
    };
    app.tree_area = Rect::default();
    app.diff_area = Rect::default();
    if tree.width > 0 {
        draw_tree(frame, app, tree);
    }
    if diff.width > 0 {
        draw_diff(frame, app, diff);
    }
    draw_footer(frame, app, footer);
    if app.help {
        draw_help(frame, area);
    }
}

fn pane(title: String, focused: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(if focused {
            ACCENT
        } else {
            Color::Rgb(57, 72, 91)
        }))
}

fn draw_tree(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = pane(
        format!(" Files ({}) · XY ", app.files.len()),
        app.focus == Focus::Tree,
    );
    let inner = block.inner(area);
    app.tree_area = inner;
    frame.render_widget(block, area);
    let rows = app.tree_rows();
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new("No changed files").style(Style::default().fg(MUTED)),
            inner,
        );
        return;
    }
    if let Some(cursor) = rows
        .iter()
        .position(|row| Some(&row.path) == app.tree_cursor.as_ref())
    {
        if cursor < app.tree_offset {
            app.tree_offset = cursor;
        }
        if cursor >= app.tree_offset + inner.height as usize {
            app.tree_offset = cursor.saturating_sub(inner.height.saturating_sub(1) as usize);
        }
    }
    app.tree_offset = app
        .tree_offset
        .min(rows.len().saturating_sub(inner.height as usize));
    for (line, row) in rows
        .iter()
        .skip(app.tree_offset)
        .take(inner.height as usize)
        .enumerate()
    {
        let cursor = Some(&row.path) == app.tree_cursor.as_ref();
        let selected = Some(&row.path) == app.selected.as_ref();
        let style = if cursor {
            Style::default().bg(Color::Rgb(37, 55, 73)).bold()
        } else {
            Style::default()
        };
        let name = row
            .path
            .file_name()
            .map(|s| display_path(s.as_ref()))
            .unwrap_or_default();
        let mut spans = vec![Span::raw(if selected { "▸" } else { " " })];
        if row.directory {
            spans.push(Span::styled(
                format!(
                    "   {}{} {name}/",
                    "  ".repeat(row.depth),
                    if app.collapsed.contains(&row.path) {
                        "▸"
                    } else {
                        "▾"
                    }
                ),
                Style::default().fg(ACCENT),
            ));
        } else if let Some(index) = row.file_index {
            let file = &app.files[index];
            spans.push(Span::styled(
                format!(
                    "{}{} ",
                    visible_status(file.index),
                    visible_status(file.worktree)
                ),
                Style::default().fg(status_color(file.marker(app.view))),
            ));
            spans.push(Span::raw(format!("{}  {name}", "  ".repeat(row.depth))));
            if let Some(original) = &file.original {
                spans.push(Span::styled(
                    format!(" ← {}", display_path(original)),
                    Style::default().fg(MUTED),
                ));
            }
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)).style(style),
            Rect::new(inner.x, inner.y + line as u16, inner.width, 1),
        );
    }
}

fn visible_status(status: char) -> char {
    if status == ' ' { '·' } else { status }
}

fn status_color(status: char) -> Color {
    match status {
        'A' | '?' => Color::LightGreen,
        'D' => Color::LightRed,
        'R' | 'C' => Color::LightCyan,
        'U' => Color::LightMagenta,
        _ => Color::Yellow,
    }
}

fn draw_diff(frame: &mut Frame, app: &mut App, area: Rect) {
    let split = match app.layout {
        LayoutMode::Auto => area.width >= 104,
        LayoutMode::Split => true,
        LayoutMode::Unified => false,
    };
    app.update_split(split);
    let name = app
        .selected
        .as_ref()
        .map(|path| display_path(path))
        .unwrap_or_else(|| "Changes".into());
    let block = pane(
        format!(
            " {name} · {} ",
            if split { "side by side" } else { "unified" }
        ),
        app.focus == Focus::Diff,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }
    let labels = match app.view {
        View::All => ("HEAD", "Working tree"),
        View::Staged => ("HEAD", "Index"),
        View::Unstaged => ("Index", "Working tree"),
    };
    if split {
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(Rect { height: 1, ..inner });
        frame.render_widget(
            Paragraph::new(format!("  {} (old)", labels.0)).style(Style::default().fg(MUTED)),
            left,
        );
        frame.render_widget(
            Paragraph::new(format!("  {} (new)", labels.1)).style(Style::default().fg(MUTED)),
            right,
        );
    } else {
        frame.render_widget(
            Paragraph::new(format!("  {} → {}", labels.0, labels.1))
                .style(Style::default().fg(MUTED)),
            Rect { height: 1, ..inner },
        );
    }
    let content = Rect::new(inner.x, inner.y + 1, inner.width, inner.height - 1);
    app.diff_area = content;
    let Some(diff) = &app.diff else {
        let message = if app.selected.is_some() {
            "Loading diff…"
        } else if app.files.is_empty() {
            "Working tree clean. Watching for changes…"
        } else {
            "Select a file to read its diff."
        };
        frame.render_widget(
            Paragraph::new(message)
                .style(Style::default().fg(MUTED))
                .wrap(Wrap { trim: false }),
            content,
        );
        return;
    };
    if let Some(summary) = &diff.summary {
        frame.render_widget(
            Paragraph::new(summary.as_str())
                .style(Style::default().fg(MUTED))
                .wrap(Wrap { trim: false }),
            content,
        );
        return;
    }
    let offset = app.position.vertical.min(diff.len(split).saturating_sub(1));
    let digits = diff
        .rows
        .iter()
        .flat_map(|row| [row.old, row.new])
        .flatten()
        .max()
        .unwrap_or(1)
        .to_string()
        .len()
        .max(4) as u16;
    for line in 0..content.height {
        let index = offset + line as usize;
        let row_area = Rect::new(content.x, content.y + line, content.width, 1);
        if split {
            let Some(pair) = diff.pairs.get(index) else {
                break;
            };
            if let Some(left) = pair.left
                && matches!(diff.rows[left].kind, Kind::Hunk | Kind::Meta)
            {
                draw_row(
                    frame,
                    &diff.rows[left],
                    row_area,
                    None,
                    digits,
                    app.position.horizontal,
                    &app.search,
                );
                continue;
            }
            let left_width = row_area.width / 2;
            let left = Rect {
                width: left_width.saturating_sub(1),
                ..row_area
            };
            let right = Rect::new(
                row_area.x + left_width,
                row_area.y,
                row_area.width - left_width,
                1,
            );
            if left_width > 0 {
                frame.render_widget(
                    Paragraph::new("│").style(Style::default().fg(MUTED)),
                    Rect::new(row_area.x + left_width - 1, row_area.y, 1, 1),
                );
            }
            if let Some(index) = pair.left {
                draw_row(
                    frame,
                    &diff.rows[index],
                    left,
                    Some(false),
                    digits,
                    app.position.horizontal,
                    &app.search,
                );
            }
            if let Some(index) = pair.right {
                draw_row(
                    frame,
                    &diff.rows[index],
                    right,
                    Some(true),
                    digits,
                    app.position.horizontal,
                    &app.search,
                );
            }
        } else if let Some(row) = diff.rows.get(index) {
            draw_row(
                frame,
                row,
                row_area,
                None,
                digits,
                app.position.horizontal,
                &app.search,
            );
        } else {
            break;
        }
    }
}

fn draw_row(
    frame: &mut Frame,
    row: &Row,
    area: Rect,
    side: Option<bool>,
    digits: u16,
    horizontal: u16,
    search: &str,
) {
    let base = row_style(row.kind);
    frame.render_widget(Block::default().style(base), area);
    if matches!(row.kind, Kind::Hunk | Kind::Meta) {
        frame.render_widget(
            Paragraph::new(display_text(&row.text))
                .style(base)
                .scroll((0, horizontal)),
            area,
        );
        return;
    }
    let number = |value: Option<usize>| value.map(|n| n.to_string()).unwrap_or_default();
    let marker = match row.kind {
        Kind::Add => '+',
        Kind::Delete => '-',
        _ => ' ',
    };
    let width = digits as usize;
    let gutter = if let Some(right) = side {
        format!(
            "{:>width$} {marker} ",
            number(if right { row.new } else { row.old })
        )
    } else {
        format!(
            "{:>width$} {:>width$} {marker} ",
            number(row.old),
            number(row.new)
        )
    };
    let gutter_width = (gutter.len() as u16).min(area.width);
    frame.render_widget(
        Paragraph::new(gutter).style(base.fg(MUTED)),
        Rect {
            width: gutter_width,
            ..area
        },
    );
    let text_area = Rect::new(area.x + gutter_width, area.y, area.width - gutter_width, 1);
    frame.render_widget(
        Paragraph::new(styled_text(row, search))
            .style(base)
            .scroll((0, horizontal)),
        text_area,
    );
}

fn row_style(kind: Kind) -> Style {
    match kind {
        Kind::Add => Style::default().bg(ADDED).fg(Color::Rgb(187, 238, 203)),
        Kind::Delete => Style::default().bg(DELETED).fg(Color::Rgb(255, 196, 204)),
        Kind::Hunk => Style::default().bg(Color::Rgb(26, 39, 56)).fg(ACCENT),
        Kind::Meta => Style::default().fg(MUTED).italic(),
        Kind::Context => Style::default().fg(FOREGROUND),
    }
}

fn styled_text(row: &Row, search: &str) -> Line<'static> {
    let base = row_style(row.kind);
    let mut spans = Vec::new();
    let matches: Vec<_> = if search.is_empty() {
        Vec::new()
    } else {
        row.text
            .to_ascii_lowercase()
            .match_indices(&search.to_ascii_lowercase())
            .map(|(start, text)| start..start + text.len())
            .collect()
    };
    let mut text = String::new();
    let mut previous = base;
    for (index, c) in row.text.char_indices() {
        let style = if matches.iter().any(|range| range.contains(&index)) {
            base.bg(Color::Yellow).fg(Color::Black).bold()
        } else if row.emphasis.iter().any(|range| range.contains(&index)) {
            base.bg(if row.kind == Kind::Add {
                ADDED_EMPHASIS
            } else {
                DELETED_EMPHASIS
            })
            .bold()
        } else {
            base
        };
        if style != previous && !text.is_empty() {
            spans.push(Span::styled(std::mem::take(&mut text), previous));
        }
        text.push_str(&display_text(&c.to_string()));
        previous = style;
    }
    if !text.is_empty() {
        spans.push(Span::styled(text, previous));
    }
    Line::from(spans)
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let (message, color) = if let Some(input) = &app.search_input {
        (format!(" /{input}█  Enter search · Esc cancel"), ACCENT)
    } else if let Some(error) = &app.error {
        (format!(" {}", display_text(error)), Color::LightRed)
    } else if let Some(notice) = &app.notice {
        (format!(" {notice}"), Color::Yellow)
    } else {
        (
            format!(
                " {} · layout {} · row {} · X=index Y=worktree",
                app.view.comparison(),
                app.layout.label(),
                app.position.vertical + 1
            ),
            MUTED,
        )
    };
    frame.render_widget(
        Paragraph::new(message).style(Style::default().fg(color)),
        Rect {
            height: area.height.min(1),
            ..area
        },
    );
    if area.height > 1 {
        frame.render_widget(Paragraph::new(" Tab focus  hjkl move  [/] hunks  / search  v layout  t tree  r refresh  ? help  q quit").style(Style::default().fg(MUTED)), Rect { y: area.y + 1, height: 1, ..area });
    }
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let width = area.width.min(76);
    let height = area.height.min(25);
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, popup);
    let text = "Read-only live Git diff viewer\n\n1 / 2 / 3     All / Staged / Unstaged\nArrows, hjkl  Navigate tree or scroll diff\nEnter         Toggle directory / open file\nTab           Switch pane focus\n[ / ]         Previous / next hunk\n/             Search files or diff in the focused pane\nEnter / Esc   Submit / cancel search (all typing is captured)\nn / N         Next / previous match\nv             Cycle auto / side by side / unified\nt             Hide / show tree\nPgUp / PgDn   Scroll a page\ng / G         First / last row\nMouse wheel   Scroll the pane under the pointer\nr             Refresh now\nq             Quit (or close this help)\n\nXY: index / working tree; · clean, ? untracked, U conflict.\nAll keeps staged entries even when net HEAD changes are zero.\nAuto uses two columns when the diff pane is at least 104 wide.\n\nEsc, Enter, or ? to close";
    frame.render_widget(
        Paragraph::new(text)
            .block(pane(" Help ".into(), true))
            .style(Style::default().bg(BACKGROUND).fg(FOREGROUND))
            .wrap(Wrap { trim: false }),
        popup,
    );
}
