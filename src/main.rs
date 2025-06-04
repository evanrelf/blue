mod defer;
mod display_width;
mod graphemes;

use crate::{
    defer::defer,
    display_width::DisplayWidth as _,
    graphemes::{floor_grapheme_boundary, next_grapheme_boundary, prev_grapheme_boundary},
};
use camino::{Utf8Path, Utf8PathBuf};
use clap::Parser as _;
use crop::Rope;
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen},
};
use pathdiff::diff_utf8_paths;
use ratatui::prelude::*;
use std::{cmp::min, env, fs, io, iter::zip, process::ExitCode};

#[derive(clap::Parser)]
struct Args {
    file: Option<Utf8PathBuf>,
}

fn main() -> anyhow::Result<ExitCode> {
    let args = Args::parse();

    let mut terminal = ratatui::init();
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;

    let _guard = defer(|| {
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        ratatui::restore();
    });

    let mut editor = if let Some(path) = args.file {
        Editor::open(path)?
    } else {
        Editor::new()?
    };

    loop {
        terminal.draw(|frame| render(&editor, frame.area(), frame.buffer_mut()))?;
        let event = crossterm::event::read()?;
        if matches!(event, Event::Resize(_, _)) {
            continue;
        }
        update(&mut editor, &event)?;
        if let Some(exit_code) = editor.exit_code {
            return Ok(exit_code);
        }
    }
}

const DARK_YELLOW: Color = Color::Rgb(0xff, 0xd3, 0x3d);

fn render(editor: &Editor, area: Rect, buffer: &mut Buffer) {
    let [status_bar, text] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);
    render_status_bar(editor, status_bar, buffer);
    render_text(editor, text, buffer);
    render_cursor(editor, text, buffer);
}

fn render_status_bar(editor: &Editor, area: Rect, buffer: &mut Buffer) {
    let mode = match editor.mode {
        Mode::Normal => "n",
        Mode::Insert => "i",
    };
    let path = match &editor.path {
        None => String::from("*scratch*"),
        Some(path) => match diff_utf8_paths(path, &editor.pwd) {
            None => path.to_string(),
            Some(path) => path.to_string(),
        },
    };
    let modified = if editor.modified { " [+]" } else { "" };
    let cursor = editor.cursor;
    let status_bar = format!("{mode} {path}{modified} {cursor}");
    Text::raw(status_bar).underlined().render(area, buffer);
}

fn render_text(editor: &Editor, area: Rect, buffer: &mut Buffer) {
    for (line, row) in zip(
        editor.text.lines().skip(editor.vertical_scroll),
        area.rows(),
    ) {
        Text::raw(line.to_string()).render(row, buffer);
    }
}

fn render_cursor(editor: &Editor, area: Rect, buffer: &mut Buffer) {
    let Some(area) = byte_offset_to_area(&editor.text, editor.cursor, editor.vertical_scroll, area)
    else {
        return;
    };
    buffer.set_style(area, Style::new().bg(DARK_YELLOW));
}

fn byte_offset_to_area(
    rope: &Rope,
    byte_offset: usize,
    vertical_scroll: usize,
    area: Rect,
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

fn update(editor: &mut Editor, event: &Event) -> anyhow::Result<()> {
    match event {
        Event::Key(key) => match editor.mode {
            Mode::Normal => match (key.modifiers, key.code) {
                (m, KeyCode::Char('h')) if m == KeyModifiers::NONE => editor.move_left(1),
                (m, KeyCode::Char('l')) if m == KeyModifiers::NONE => editor.move_right(1),
                (m, KeyCode::Char('d')) if m == KeyModifiers::NONE => editor.delete_after(),
                (m, KeyCode::Char('i')) if m == KeyModifiers::NONE => editor.mode = Mode::Insert,
                (m, KeyCode::Char('s')) if m == KeyModifiers::CONTROL => editor.save()?,
                (m, KeyCode::Char('c')) if m == KeyModifiers::CONTROL && !editor.modified => {
                    editor.exit_code = Some(ExitCode::SUCCESS);
                }
                _ => {}
            },
            Mode::Insert => match (key.modifiers, key.code) {
                (m, KeyCode::Char(char)) if m == KeyModifiers::NONE => {
                    editor.insert(&char.to_string());
                }
                (m, KeyCode::Char(char)) if m == KeyModifiers::SHIFT => {
                    editor.insert(&char.to_string());
                }
                (m, KeyCode::Backspace) if m == KeyModifiers::NONE => editor.delete_before(),
                (m, KeyCode::Esc) if m == KeyModifiers::NONE => editor.mode = Mode::Normal,
                (m, KeyCode::Char('s')) if m == KeyModifiers::CONTROL => editor.save()?,
                (m, KeyCode::Char('c')) if m == KeyModifiers::CONTROL && !editor.modified => {
                    editor.exit_code = Some(ExitCode::SUCCESS);
                }
                _ => {}
            },
        },
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::ScrollUp => editor.scroll_up(3),
            MouseEventKind::ScrollDown => editor.scroll_down(3),
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
    cursor: usize,
    vertical_scroll: usize,
    mode: Mode,
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
        if !self.modified {
            return Ok(());
        }
        if let Some(path) = &self.path {
            let bytes = self.text.bytes().collect::<Vec<_>>();
            fs::write(path, bytes)?;
            self.modified = false;
        }
        Ok(())
    }

    fn move_left(&mut self, count: usize) {
        debug_assert!(self.text.is_grapheme_boundary(self.cursor));
        for _ in 0..count {
            match prev_grapheme_boundary(&self.text.byte_slice(..), self.cursor) {
                Some(prev) if self.cursor != prev => self.cursor = prev,
                _ => break,
            }
        }
    }

    fn move_right(&mut self, count: usize) {
        debug_assert!(self.text.is_grapheme_boundary(self.cursor));
        for _ in 0..count {
            match next_grapheme_boundary(&self.text.byte_slice(..), self.cursor) {
                Some(next) if self.cursor != next => self.cursor = next,
                _ => break,
            }
        }
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
        self.text.insert(self.cursor, text);
        self.cursor += text.len();
        self.modified = true;
    }

    fn delete_before(&mut self) {
        if let Some(grapheme) = self.text.byte_slice(..self.cursor).graphemes().next_back() {
            let start = self.cursor - grapheme.len();
            let end = self.cursor;
            self.text.delete(start..end);
            self.cursor = start;
            self.modified = true;
        }
    }

    fn delete_after(&mut self) {
        if let Some(grapheme) = self.text.byte_slice(self.cursor..).graphemes().next() {
            let start = self.cursor;
            let end = start + grapheme.len();
            self.text.delete(start..end);
            self.modified = true;
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
            cursor: 0,
            vertical_scroll: 0,
            mode: Mode::Normal,
            exit_code: None,
        })
    }
}

enum Mode {
    Normal,
    Insert,
}
