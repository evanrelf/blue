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
        DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers,
        KeyboardEnhancementFlags, MouseButton, MouseEventKind, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen},
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
        EnterAlternateScreen,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES),
        EnableMouseCapture,
    )?;

    let _guard = defer(|| {
        let _ = execute!(
            io::stdout(),
            LeaveAlternateScreen,
            PopKeyboardEnhancementFlags,
            DisableMouseCapture,
        );
        ratatui::restore();
    });

    let mut editor = if let Some(path) = args.file {
        Editor::open(path)?
    } else {
        Editor::new()?
    };

    let mut area = Rect::default();

    loop {
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
            return Ok(exit_code);
        }
    }
}

const LIGHT_YELLOW: Color = Color::Rgb(0xff, 0xf5, 0xb1);

const DARK_YELLOW: Color = Color::Rgb(0xff, 0xd3, 0x3d);

struct Areas {
    status_bar: Rect,
    text: Rect,
}

impl Areas {
    fn new(area: Rect) -> Self {
        let [status_bar, text] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);
        Self { status_bar, text }
    }
}

fn render(editor: &Editor, area: Rect, buffer: &mut Buffer) {
    let areas = Areas::new(area);
    render_status_bar(editor, areas.status_bar, buffer);
    render_text(editor, areas.text, buffer);
    render_cursor(editor, areas.text, buffer);
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
    let anchor = editor.anchor;
    let head = editor.head;
    let status_bar = format!("{mode} {path}{modified} {anchor}-{head}");
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
    // anchor
    if let Some(area) =
        byte_offset_to_area(&editor.text, editor.vertical_scroll, area, editor.anchor)
    {
        buffer.set_style(area, Style::new().bg(LIGHT_YELLOW));
    }
    // head
    if let Some(area) = byte_offset_to_area(&editor.text, editor.vertical_scroll, area, editor.head)
    {
        buffer.set_style(area, Style::new().bg(DARK_YELLOW));
    }
}

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

fn position_to_byte_offset(
    rope: &Rope,
    vertical_scroll: usize,
    area: Rect,
    position: Position,
) -> Option<usize> {
    if !area.contains(position) {
        return None;
    }

    let column = usize::from(position.x - area.x);
    let row = usize::from(position.y - area.y) + vertical_scroll;

    if row >= rope.line_len() {
        return Some(rope.byte_len());
    }

    // TODO: Handle multi-byte and wide graphemes

    let line_length = rope.line(row).byte_len();

    let byte_offset = rope.byte_of_line(row) + min(line_length, column);

    Some(byte_offset)
}

fn update(editor: &mut Editor, area: Rect, event: &Event) -> anyhow::Result<()> {
    let areas = Areas::new(area);
    match event {
        Event::Key(key) => match editor.mode {
            Mode::Normal => match (key.modifiers, key.code) {
                (m, KeyCode::Char('h')) if m == KeyModifiers::NONE => editor.move_left(1),
                (m, KeyCode::Char('l')) if m == KeyModifiers::NONE => editor.move_right(1),
                (m, KeyCode::Char('h' | 'H')) if m == KeyModifiers::SHIFT => editor.extend_left(1),
                (m, KeyCode::Char('l' | 'L')) if m == KeyModifiers::SHIFT => editor.extend_right(1),
                (m, KeyCode::Char(';')) if m == KeyModifiers::NONE => editor.reduce(),
                (m, KeyCode::Char(';')) if m == KeyModifiers::ALT => editor.flip(),
                (m, KeyCode::Char(';')) if m == KeyModifiers::SHIFT | KeyModifiers::ALT => {
                    editor.flip_forward();
                }
                (m, KeyCode::Char('d')) if m == KeyModifiers::NONE => editor.delete(),
                (m, KeyCode::Char('i')) if m == KeyModifiers::NONE => editor.mode = Mode::Insert,
                (m, KeyCode::Char('s')) if m == KeyModifiers::CONTROL => editor.save()?,
                (m, KeyCode::Char('c')) if m == KeyModifiers::CONTROL && !editor.modified => {
                    editor.exit_code = Some(ExitCode::SUCCESS);
                }
                (m, KeyCode::Char('c')) if m == KeyModifiers::SHIFT | KeyModifiers::CONTROL => {
                    editor.exit_code = Some(ExitCode::FAILURE);
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
                (m, KeyCode::Char('c')) if m == KeyModifiers::SHIFT | KeyModifiers::CONTROL => {
                    editor.exit_code = Some(ExitCode::FAILURE);
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
                    editor.anchor = byte_offset;
                    editor.head = byte_offset;
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(byte_offset) = position_to_byte_offset(
                    &editor.text,
                    editor.vertical_scroll,
                    areas.text,
                    Position::new(mouse.column, mouse.row),
                ) {
                    editor.head = byte_offset;
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

    fn extend_left(&mut self, count: usize) {
        debug_assert!(self.text.is_grapheme_boundary(self.head));
        for _ in 0..count {
            match prev_grapheme_boundary(&self.text.byte_slice(..), self.head) {
                Some(prev) if self.head != prev => self.head = prev,
                _ => break,
            }
        }
    }

    fn extend_right(&mut self, count: usize) {
        debug_assert!(self.text.is_grapheme_boundary(self.head));
        for _ in 0..count {
            match next_grapheme_boundary(&self.text.byte_slice(..), self.head) {
                Some(next) if self.head != next => self.head = next,
                _ => break,
            }
        }
    }

    fn move_left(&mut self, count: usize) {
        self.extend_left(count);
        self.reduce();
    }

    fn move_right(&mut self, count: usize) {
        self.extend_right(count);
        self.reduce();
    }

    fn flip(&mut self) {
        mem::swap(&mut self.anchor, &mut self.head);
    }

    fn flip_forward(&mut self) {
        if self.anchor > self.head {
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
        self.modified = true;
    }

    fn delete_before(&mut self) {
        if let Some(grapheme) = self.text.byte_slice(..self.head).graphemes().next_back() {
            let start = self.head - grapheme.len();
            let end = self.head;
            self.text.delete(start..end);
            self.head = start;
            self.modified = true;
            debug_assert!(self.text.is_grapheme_boundary(self.anchor));
            debug_assert!(self.text.is_grapheme_boundary(self.head));
        }
    }

    fn delete(&mut self) {
        let start = min(self.anchor, self.head);
        let end = max(self.anchor, self.head);
        self.text.delete(start..end);
        self.head = self.anchor;
        debug_assert!(self.text.is_grapheme_boundary(self.anchor));
        debug_assert!(self.text.is_grapheme_boundary(self.head));
    }

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
