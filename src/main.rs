mod defer;
mod display_width;
mod graphemes;

use crate::{
    defer::defer,
    display_width::DisplayWidth as _,
    graphemes::{
        ceil_grapheme_boundary, floor_grapheme_boundary, next_grapheme_boundary,
        prev_grapheme_boundary,
    },
};
use camino::{Utf8Path, Utf8PathBuf};
use clap::Parser as _;
use crop::Rope;
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers,
        KeyboardEnhancementFlags, MouseButton, MouseEventKind, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{Clear, ClearType},
};
use pathdiff::diff_utf8_paths;
use ratatui::prelude::*;
use std::{
    cmp::{max, min},
    env, fs, io,
    iter::zip,
    mem,
    process::ExitCode,
};

#[derive(clap::Parser)]
struct Args {
    file: Option<Utf8PathBuf>,
}

fn main() -> anyhow::Result<ExitCode> {
    let args = Args::parse();

    let mut terminal = ratatui::init();
    execute!(
        io::stdout(),
        Clear(ClearType::All),
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES),
        EnableMouseCapture,
    )?;
    let _guard = defer(|| {
        let _ = execute!(
            io::stdout(),
            PopKeyboardEnhancementFlags,
            DisableMouseCapture,
        );
    });

    let mut editor = if let Some(path) = args.file {
        Editor::open(path)?
    } else {
        Editor::new()?
    };

    let mut area = Rect::default();

    let exit_code = loop {
        terminal.draw(|frame| {
            area = frame.area();
            render(&editor, area, frame.buffer_mut());
        })?;
        let event = crossterm::event::read()?;
        if matches!(event, Event::Resize(_, _)) {
            continue;
        }
        update(&mut editor, area, &event)?;
        if let Some(exit_code) = editor.exit_code {
            break exit_code;
        }
    };

    ratatui::restore();

    Ok(exit_code)
}

const LIGHT_RED: Color = Color::Rgb(0xff, 0xdc, 0xe0);

const LIGHT_YELLOW: Color = Color::Rgb(0xff, 0xf5, 0xb1);

const DARK_YELLOW: Color = Color::Rgb(0xff, 0xd3, 0x3d);

struct Areas {
    status_bar: Rect,
    line_numbers: Rect,
    text: Rect,
}

impl Areas {
    fn new(text: &Rope, area: Rect) -> Self {
        let line_numbers_width = {
            let n = text.line_len();
            let digits = 1 + max(1, n).ilog10();
            u16::try_from(max(2, digits) + 1)
                .expect("Line number width should always be very small")
        };
        let [status_bar, main] = Layout::vertical([
            // status bar
            Constraint::Length(1),
            // line_numbers + text
            Constraint::Fill(1),
        ])
        .areas(area);
        let [line_numbers, text] = Layout::horizontal([
            // line_numbers
            Constraint::Length(line_numbers_width),
            // fill
            Constraint::Fill(1),
        ])
        .areas(main);
        Self {
            status_bar,
            line_numbers,
            text,
        }
    }
}

fn render(editor: &Editor, area: Rect, buffer: &mut Buffer) {
    let areas = Areas::new(&editor.text, area);
    render_status_bar(editor, areas.status_bar, buffer);
    render_line_numbers(editor, areas.line_numbers, buffer);
    render_text(editor, areas.text, buffer);
    render_selection(editor, areas.text, buffer);
}

fn render_status_bar(editor: &Editor, area: Rect, buffer: &mut Buffer) {
    if let Some(error) = &editor.error {
        let status_bar = format!("Error: {error}");
        Line::raw(status_bar)
            .underlined()
            .bg(LIGHT_RED)
            .render(area, buffer);
    } else if let Mode::Command = editor.mode {
        let status_bar = format!(":{}", editor.command);
        Line::raw(status_bar).underlined().render(area, buffer);
        let cursor_x = area.x
            + 1
            + u16::try_from(
                editor
                    .command
                    .byte_slice(..editor.command_cursor)
                    .display_width(),
            )
            .expect("Command length should not exceed `u16::MAX`");
        if let Some(cell) = buffer.cell_mut((cursor_x, area.y)) {
            cell.set_bg(DARK_YELLOW);
        }
    } else {
        let mode = match editor.mode {
            Mode::Normal => "n",
            Mode::Insert => "i",
            Mode::Command => unreachable!(),
        };
        let path = match &editor.path {
            None => String::from("*scratch*"),
            Some(path) => match diff_utf8_paths(path, &editor.pwd) {
                None => path.to_string(),
                Some(path) => path.to_string(),
            },
        };
        let modified = if editor.modified { " [+]" } else { "" };
        let anchor = editor.anchor;
        let head = editor.head;
        let status_bar = format!("{mode} {path}{modified} {anchor}-{head}");
        Line::raw(status_bar).underlined().render(area, buffer);
    }
}

fn render_line_numbers(editor: &Editor, area: Rect, buffer: &mut Buffer) {
    for (line_number, row) in zip(
        editor.vertical_scroll + 1..=editor.text.line_len(),
        area.rows(),
    ) {
        Line::raw(format!("{line_number}│"))
            .right_aligned()
            .render(row, buffer);
    }
}

fn render_text(editor: &Editor, area: Rect, buffer: &mut Buffer) {
    for (line, row) in zip(
        editor.text.lines().skip(editor.vertical_scroll),
        area.rows(),
    ) {
        Line::raw(line.to_string().replace('\t', "        ")).render(row, buffer);
    }
}

fn render_selection(editor: &Editor, area: Rect, buffer: &mut Buffer) {
    if editor.anchor != editor.head {
        let start = min(editor.anchor, editor.head);
        let end = max(editor.anchor, editor.head);
        let start_line = editor.text.line_of_byte(start);
        let end_line = editor.text.line_of_byte(end.saturating_sub(1));
        for line_index in start_line..=end_line {
            let Some(mut line_area) =
                line_index_to_area(&editor.text, editor.vertical_scroll, area, line_index)
            else {
                continue;
            };
            if line_index == start_line {
                if let Some(start_area) =
                    byte_offset_to_area(&editor.text, editor.vertical_scroll, area, start)
                {
                    let delta = start_area.x - line_area.x;
                    line_area.x += delta;
                    line_area.width -= delta;
                } else {
                    // TODO: We continue here because we know the range start is off the screen to
                    // the right. Once horizontal scrolling is added, we'll need to handle when the
                    // range is off the screen to the left. `byte_offset_to_area` doesn't say which
                    // direction the index is off screen.
                    continue;
                }
            }
            #[expect(clippy::collapsible_if)]
            if line_index == end_line {
                if let Some(end_area) = byte_offset_to_area(
                    &editor.text,
                    editor.vertical_scroll,
                    area,
                    end.saturating_sub(1),
                ) {
                    let delta = line_area.right() - end_area.right();
                    line_area.width -= delta;
                }
            }
            buffer.set_style(line_area, Style::new().bg(LIGHT_YELLOW));
        }
    }
    let head = if editor.anchor < editor.head {
        prev_grapheme_boundary(&editor.text.byte_slice(..), editor.head).unwrap_or(editor.head)
    } else {
        editor.head
    };
    if let Some(area) = byte_offset_to_area(&editor.text, editor.vertical_scroll, area, head) {
        buffer.set_style(area, Style::new().bg(DARK_YELLOW));
    }
}

// TODO: Optimize
fn byte_offset_to_area(
    rope: &Rope,
    vertical_scroll: usize,
    area: Rect,
    byte_offset: usize,
) -> Option<Rect> {
    if byte_offset > rope.byte_len() {
        return None;
    }

    let line_offset = rope.line_of_byte(byte_offset);

    if vertical_scroll > line_offset {
        return None;
    }

    let y = area.y + u16::try_from(line_offset - vertical_scroll).unwrap();

    if !(area.top()..area.bottom()).contains(&y) {
        return None;
    }

    let line_byte_offset = rope.byte_of_line(line_offset);

    let byte_offset = floor_grapheme_boundary(&rope.byte_slice(..), byte_offset);

    let prefix_width = rope
        .byte_slice(line_byte_offset..byte_offset)
        .display_width();

    // TODO: When horizontal scroll is introduced, still return portion of rect that is visible.
    // Even if it starts to the left of the area, it might be wide enough to peek into the viewport.
    let x = area.x + u16::try_from(prefix_width).unwrap();

    if !(area.left()..area.right()).contains(&x) {
        return None;
    }

    let width = if rope.byte_len() == byte_offset {
        // Cursor at EOF
        1
    } else if let Some(grapheme) = rope.byte_slice(byte_offset..).graphemes().next() {
        u16::try_from(grapheme.as_ref().display_width()).unwrap()
    } else {
        // We're at EOF, but we already checked for that
        unreachable!()
    };

    Some(Rect {
        x,
        y,
        width,
        height: 1,
    })
}

fn line_index_to_area(
    rope: &Rope,
    vertical_scroll: usize,
    area: Rect,
    line_index: usize,
) -> Option<Rect> {
    if vertical_scroll > line_index {
        return None;
    }

    if line_index >= rope.line_len() {
        return None;
    }

    let x = area.x;

    let y = area.y + u16::try_from(line_index - vertical_scroll).unwrap();

    if !(area.top()..area.bottom()).contains(&y) {
        return None;
    }

    let line = rope.line_slice(line_index..=line_index);

    let width = u16::try_from(line.display_width()).unwrap();

    Some(Rect {
        x,
        y,
        width,
        height: 1,
    })
}

fn position_to_byte_offset(
    rope: &Rope,
    vertical_scroll: usize,
    area: Rect,
    position: Position,
) -> Option<usize> {
    if !area.contains(position) {
        return None;
    }

    let target_column = usize::from(position.x - area.x);
    let row = usize::from(position.y - area.y) + vertical_scroll;

    if row >= rope.line_len() {
        return Some(rope.byte_len());
    }

    let mut current_column = 0;
    let mut byte_offset = rope.byte_of_line(row);

    for grapheme in rope.line(row).graphemes() {
        let grapheme_width = grapheme.as_ref().display_width();
        if current_column + grapheme_width > target_column {
            break;
        }
        current_column += grapheme_width;
        byte_offset += grapheme.len();
    }

    Some(byte_offset)
}

#[expect(clippy::too_many_lines)]
fn update(editor: &mut Editor, area: Rect, event: &Event) -> anyhow::Result<()> {
    editor.error = None;
    let areas = Areas::new(&editor.text, area);
    #[allow(clippy::match_same_arms)]
    match event {
        Event::Key(key) => match editor.mode {
            Mode::Normal => match (key.modifiers, key.code) {
                (m, KeyCode::Char('h')) if m == KeyModifiers::NONE => editor.move_left(1),
                (m, KeyCode::Char('l')) if m == KeyModifiers::NONE => editor.move_right(1),
                (m, KeyCode::Char('k')) if m == KeyModifiers::NONE => editor.move_up(1),
                (m, KeyCode::Char('j')) if m == KeyModifiers::NONE => editor.move_down(1),
                (m, KeyCode::Char('h' | 'H')) if m == KeyModifiers::SHIFT => editor.extend_left(1),
                (m, KeyCode::Char('l' | 'L')) if m == KeyModifiers::SHIFT => editor.extend_right(1),
                (m, KeyCode::Char('k' | 'K')) if m == KeyModifiers::SHIFT => editor.extend_up(1),
                (m, KeyCode::Char('j' | 'J')) if m == KeyModifiers::SHIFT => editor.extend_down(1),
                (m, KeyCode::Char(';')) if m == KeyModifiers::NONE => editor.reduce(),
                (m, KeyCode::Char(';')) if m == KeyModifiers::ALT => editor.flip(),
                (m, KeyCode::Char(';')) if m == KeyModifiers::SHIFT | KeyModifiers::ALT => {
                    editor.flip_forward();
                }
                (m, KeyCode::Char('d')) if m == KeyModifiers::NONE => editor.delete(),
                (m, KeyCode::Char('c')) if m == KeyModifiers::NONE => {
                    editor.delete();
                    editor.mode = Mode::Insert;
                }
                (m, KeyCode::Char('i')) if m == KeyModifiers::NONE => {
                    editor.reduce();
                    editor.mode = Mode::Insert;
                }
                (m, KeyCode::Char(':')) if m == KeyModifiers::NONE => {
                    editor.command = Rope::new();
                    editor.command_cursor = 0;
                    editor.mode = Mode::Command;
                }
                (m, KeyCode::Char('u')) if m == KeyModifiers::CONTROL => {
                    editor.scroll_up(usize::from(areas.text.height.saturating_sub(1) / 2));
                }
                (m, KeyCode::Char('d')) if m == KeyModifiers::CONTROL => {
                    editor.scroll_down(usize::from(areas.text.height.saturating_sub(1) / 2));
                }
                (m, KeyCode::Char('b')) if m == KeyModifiers::CONTROL => {
                    editor.scroll_up(usize::from(areas.text.height.saturating_sub(2)));
                }
                (m, KeyCode::Char('f')) if m == KeyModifiers::CONTROL => {
                    editor.scroll_down(usize::from(areas.text.height.saturating_sub(2)));
                }
                _ => {}
            },
            Mode::Insert => match (key.modifiers, key.code) {
                (m, KeyCode::Char('a')) if m == KeyModifiers::CONTROL => editor.move_line_start(),
                (m, KeyCode::Char('e')) if m == KeyModifiers::CONTROL => editor.move_line_end(),
                (m, KeyCode::Char('b')) if m == KeyModifiers::CONTROL => editor.move_left(1),
                (m, KeyCode::Char('f')) if m == KeyModifiers::CONTROL => editor.move_right(1),
                (m, KeyCode::Char(char)) if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT => {
                    editor.insert(&char.to_string());
                }
                (m, KeyCode::Tab) if m == KeyModifiers::NONE => {
                    editor.insert("\t");
                }
                (m, KeyCode::Enter) if m == KeyModifiers::NONE => {
                    editor.insert("\n");
                    editor.desired_column = None;
                }
                (m, KeyCode::Backspace) if m == KeyModifiers::NONE => editor.delete_before(),
                (m, KeyCode::Esc) if m == KeyModifiers::NONE => editor.mode = Mode::Normal,
                _ => {}
            },
            Mode::Command => match (key.modifiers, key.code) {
                (m, KeyCode::Char(char)) if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT => {
                    let string = char.to_string();
                    editor.command.insert(editor.command_cursor, &string);
                    editor.command_cursor += string.len();
                }
                (m, KeyCode::Backspace) if m == KeyModifiers::NONE => {
                    if editor.command_cursor > 0 {
                        debug_assert!(!editor.command.is_empty());
                        if let Some(prev) = prev_grapheme_boundary(
                            &editor.command.byte_slice(..),
                            editor.command_cursor,
                        ) {
                            editor.command.delete(prev..editor.command_cursor);
                            editor.command_cursor = prev;
                        }
                    } else if editor.command.is_empty() {
                        debug_assert!(editor.command_cursor == 0);
                        editor.command = Rope::new();
                        editor.command_cursor = 0;
                        editor.mode = Mode::Normal;
                    }
                }
                (m, KeyCode::Enter) if m == KeyModifiers::NONE => {
                    let command = editor.command.to_string();
                    let command = command.trim();
                    match command {
                        "write" | "w" => editor.save()?,
                        "quit" | "q" => {
                            if editor.modified {
                                editor.error = Some(String::from("Unsaved changes"));
                            } else {
                                editor.exit_code = Some(ExitCode::SUCCESS);
                            }
                        }
                        "quit!" | "q!" => editor.exit_code = Some(ExitCode::FAILURE),
                        "write-quit" | "wq" => {
                            editor.save()?;
                            editor.exit_code = Some(ExitCode::SUCCESS);
                        }
                        "" => {}
                        _ => {
                            editor.error = Some(format!("Unknown command `{command}`"));
                        }
                    }
                    editor.command = Rope::new();
                    editor.command_cursor = 0;
                    editor.mode = Mode::Normal;
                }
                (m, KeyCode::Esc) if m == KeyModifiers::NONE => {
                    editor.command = Rope::new();
                    editor.command_cursor = 0;
                    editor.mode = Mode::Normal;
                }
                _ => {}
            },
        },
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::ScrollUp => editor.scroll_up(3),
            MouseEventKind::ScrollDown => editor.scroll_down(3),
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(byte_offset) = position_to_byte_offset(
                    &editor.text,
                    editor.vertical_scroll,
                    areas.text,
                    Position::new(mouse.column, mouse.row),
                ) {
                    if editor.is_backward() {
                        editor.head = byte_offset;
                    } else {
                        editor.head =
                            ceil_grapheme_boundary(&editor.text.byte_slice(..), byte_offset + 1);
                    }
                    editor.anchor = byte_offset;
                    editor.desired_column = None;
                }
            }
            MouseEventKind::Down(MouseButton::Right)
            | MouseEventKind::Drag(MouseButton::Left | MouseButton::Right) => {
                if let Some(byte_offset) = position_to_byte_offset(
                    &editor.text,
                    editor.vertical_scroll,
                    areas.text,
                    Position::new(mouse.column, mouse.row),
                ) {
                    if editor.is_backward() {
                        editor.head = byte_offset;
                    } else {
                        editor.head =
                            ceil_grapheme_boundary(&editor.text.byte_slice(..), byte_offset + 1);
                    }
                    editor.desired_column = None;
                }
            }
            _ => {}
        },
        _ => {}
    }
    Ok(())
}

struct Editor {
    pwd: Utf8PathBuf,
    path: Option<Utf8PathBuf>,
    modified: bool,
    text: Rope,
    anchor: usize,
    head: usize,
    desired_column: Option<usize>,
    vertical_scroll: usize,
    mode: Mode,
    command: Rope,
    command_cursor: usize,
    error: Option<String>,
    exit_code: Option<ExitCode>,
}

impl Editor {
    fn new() -> anyhow::Result<Self> {
        Self::try_from(Rope::new())
    }

    fn open(path: impl AsRef<Utf8Path>) -> anyhow::Result<Self> {
        let exists = path.as_ref().try_exists()?;
        let path = if exists {
            path.as_ref().canonicalize_utf8()?
        } else {
            path.as_ref().to_path_buf()
        };
        let rope = if exists {
            let string = fs::read_to_string(&path)?;
            Rope::from(string)
        } else {
            Rope::new()
        };
        let mut editor = Self::try_from(rope)?;
        editor.path = Some(path);
        Ok(editor)
    }

    fn save(&mut self) -> anyhow::Result<()> {
        if let Some(path) = &self.path {
            let bytes = self.text.bytes().collect::<Vec<_>>();
            fs::write(path, bytes)?;
            self.modified = false;
        }
        Ok(())
    }

    fn extend_left(&mut self, count: usize) {
        debug_assert!(self.text.is_grapheme_boundary(self.head));
        for _ in 0..count {
            match prev_grapheme_boundary(&self.text.byte_slice(..), self.head) {
                Some(prev) if self.head != prev => self.head = prev,
                _ => break,
            }
        }
        self.desired_column = None;
    }

    fn extend_right(&mut self, count: usize) {
        debug_assert!(self.text.is_grapheme_boundary(self.head));
        for _ in 0..count {
            match next_grapheme_boundary(&self.text.byte_slice(..), self.head) {
                Some(next) if self.head != next => self.head = next,
                _ => break,
            }
        }
        self.desired_column = None;
    }

    fn extend_up(&mut self, count: usize) {
        for _ in 0..count {
            let current_line_index = self.text.line_of_byte(self.head);
            if current_line_index == 0 {
                break;
            }
            let target_line_index = current_line_index - 1;
            let current_line_byte_index = self.text.byte_of_line(current_line_index);
            let desired_column = self.desired_column.unwrap_or_else(|| {
                self.text
                    .byte_slice(current_line_byte_index..self.head)
                    .display_width()
            });
            self.desired_column = Some(desired_column);
            let target_line_byte_index = self.text.byte_of_line(target_line_index);
            let target_line_slice = self.text.line(target_line_index);
            let mut target_line_prefix = 0;
            let mut byte_offset = target_line_byte_index;
            for grapheme in target_line_slice.graphemes() {
                let grapheme_width = grapheme.as_ref().display_width();
                if target_line_prefix + grapheme_width > desired_column {
                    break;
                }
                target_line_prefix += grapheme_width;
                byte_offset += grapheme.len();
            }
            self.head = byte_offset;
        }
    }

    fn extend_down(&mut self, count: usize) {
        for _ in 0..count {
            let current_line_index = self.text.line_of_byte(self.head);
            let target_line_index = current_line_index + 1;
            if target_line_index >= self.text.line_len() {
                self.head = self.text.byte_len();
                break;
            }
            let current_line_byte_index = self.text.byte_of_line(current_line_index);
            let desired_column = self.desired_column.unwrap_or_else(|| {
                self.text
                    .byte_slice(current_line_byte_index..self.head)
                    .display_width()
            });
            self.desired_column = Some(desired_column);
            let target_line_byte_index = self.text.byte_of_line(target_line_index);
            let target_line_slice = self.text.line(target_line_index);
            let mut target_line_prefix = 0;
            let mut byte_offset = target_line_byte_index;
            for grapheme in target_line_slice.graphemes() {
                let grapheme_width = grapheme.as_ref().display_width();
                if target_line_prefix + grapheme_width > desired_column {
                    break;
                }
                target_line_prefix += grapheme_width;
                byte_offset += grapheme.len();
            }
            self.head = byte_offset;
        }
    }

    fn extend_line_start(&mut self) {
        let line_index = self.text.line_of_byte(self.head);
        let line_start_byte_index = self.text.byte_of_line(line_index);
        self.head = line_start_byte_index;
    }

    fn extend_line_end(&mut self) {
        let line_index = self.text.line_of_byte(self.head);
        let line_start_byte_index = self.text.byte_of_line(line_index);
        let line = self.text.line(line_index);
        let line_end_byte_index = line_start_byte_index + line.byte_len();
        self.head = line_end_byte_index;
    }

    fn move_left(&mut self, count: usize) {
        self.extend_left(count);
        self.reduce();
    }

    fn move_right(&mut self, count: usize) {
        self.extend_right(count);
        self.reduce();
    }

    fn move_up(&mut self, count: usize) {
        self.extend_up(count);
        self.reduce();
    }

    fn move_down(&mut self, count: usize) {
        self.extend_down(count);
        self.reduce();
    }

    fn move_line_start(&mut self) {
        self.extend_line_start();
        self.reduce();
    }

    fn move_line_end(&mut self) {
        self.extend_line_end();
        self.reduce();
    }

    fn is_forward(&self) -> bool {
        self.anchor <= self.head
    }

    fn is_backward(&self) -> bool {
        !self.is_forward()
    }

    fn flip(&mut self) {
        mem::swap(&mut self.anchor, &mut self.head);
    }

    fn flip_forward(&mut self) {
        if !self.is_forward() {
            self.flip();
        }
    }

    fn reduce(&mut self) {
        self.anchor = self.head;
    }

    fn scroll_up(&mut self, distance: usize) {
        debug_assert!(self.vertical_scroll < self.text.line_len());
        self.vertical_scroll = self.vertical_scroll.saturating_sub(distance);
    }

    fn scroll_down(&mut self, distance: usize) {
        debug_assert!(self.vertical_scroll < self.text.line_len());
        self.vertical_scroll = min(
            self.text.line_len().saturating_sub(1),
            self.vertical_scroll + distance,
        );
    }

    fn insert(&mut self, text: &str) {
        self.text.insert(self.head, text);
        self.head += text.len();
        self.reduce();
        self.modified = true;
    }

    fn delete_before(&mut self) {
        if let Some(grapheme) = self.text.byte_slice(..self.head).graphemes().next_back() {
            let start = self.head - grapheme.len();
            let end = self.head;
            self.text.delete(start..end);
            self.head = start;
            self.reduce();
            self.modified = true;
            debug_assert!(self.text.is_grapheme_boundary(self.anchor));
            debug_assert!(self.text.is_grapheme_boundary(self.head));
        }
    }

    fn delete(&mut self) {
        let start = min(self.anchor, self.head);
        let end = max(self.anchor, self.head);
        self.text.delete(start..end);
        self.head = start;
        self.anchor = start;
        self.modified = true;
        debug_assert!(self.text.is_grapheme_boundary(self.anchor));
        debug_assert!(self.text.is_grapheme_boundary(self.head));
    }

    #[expect(dead_code)]
    fn delete_after(&mut self) {
        if let Some(grapheme) = self.text.byte_slice(self.head..).graphemes().next() {
            let start = self.head;
            let end = start + grapheme.len();
            self.text.delete(start..end);
            self.modified = true;
            debug_assert!(self.text.is_grapheme_boundary(self.anchor));
            debug_assert!(self.text.is_grapheme_boundary(self.head));
        }
    }
}

impl TryFrom<Rope> for Editor {
    type Error = anyhow::Error;
    fn try_from(rope: Rope) -> Result<Self, Self::Error> {
        Ok(Self {
            pwd: Utf8PathBuf::try_from(env::current_dir()?)?,
            path: None,
            modified: false,
            text: rope,
            anchor: 0,
            head: 0,
            desired_column: None,
            vertical_scroll: 0,
            mode: Mode::Normal,
            command: Rope::new(),
            command_cursor: 0,
            error: None,
            exit_code: None,
        })
    }
}

enum Mode {
    Normal,
    Insert,
    Command,
}
