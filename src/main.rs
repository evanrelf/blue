mod defer;
mod graphemes;

use crate::{
    defer::defer,
    graphemes::{next_grapheme_boundary, prev_grapheme_boundary},
};
use camino::Utf8PathBuf;
use clap::Parser as _;
use crop::Rope;
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::{cmp::min, fs, io, iter::zip, process::ExitCode};

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
        Editor::new()
    };

    loop {
        terminal.draw(|frame| render(&editor, frame.area(), frame.buffer_mut()))?;
        let event = crossterm::event::read()?;
        if matches!(event, Event::Resize(_, _)) {
            continue;
        }
        update(&mut editor, &event);
        if let Some(exit_code) = editor.exit_code {
            return Ok(exit_code);
        }
    }
}

fn render(editor: &Editor, area: Rect, buffer: &mut Buffer) {
    render_text(editor, area, buffer);
    render_cursor(editor, area, buffer);
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
    // TODO: Actually render a colored rectangle
    Text::raw(format!("{}", editor.cursor)).render(area, buffer);
}

fn update(editor: &mut Editor, event: &Event) {
    match event {
        Event::Key(key) => match (key.modifiers, key.code) {
            (m, KeyCode::Char('h')) if m == KeyModifiers::NONE => editor.move_left(1),
            (m, KeyCode::Char('l')) if m == KeyModifiers::NONE => editor.move_right(1),
            (m, KeyCode::Char('c')) if m == KeyModifiers::CONTROL => {
                editor.exit_code = Some(ExitCode::FAILURE);
            }
            (m, KeyCode::Char('p')) if m == KeyModifiers::CONTROL => panic!(),
            _ => {}
        },
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::ScrollUp => editor.scroll_up(3),
            MouseEventKind::ScrollDown => editor.scroll_down(3),
            _ => {}
        },
        _ => {}
    }
}

#[derive(Default)]
struct Editor {
    text: Rope,
    cursor: usize,
    vertical_scroll: usize,
    exit_code: Option<ExitCode>,
}

impl Editor {
    fn new() -> Self {
        Self::default()
    }

    fn open(path: Utf8PathBuf) -> anyhow::Result<Self> {
        let string = fs::read_to_string(path)?;
        let rope = Rope::from(string);
        Ok(Self {
            text: rope,
            ..Self::default()
        })
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
}
