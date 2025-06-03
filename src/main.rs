mod defer;

use crate::defer::defer;
use camino::Utf8PathBuf;
use clap::Parser as _;
use crop::Rope;
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers, MouseEventKind,
    },
    execute,
};
use ratatui::prelude::*;
use std::{cmp::min, fs, io, iter::zip};

#[derive(clap::Parser)]
struct Args {
    file: Option<Utf8PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let mut terminal = ratatui::init();
    execute!(io::stdout(), EnableMouseCapture)?;

    let _guard = defer(|| {
        let _ = execute!(io::stdout(), DisableMouseCapture);
        ratatui::restore();
    });

    let mut editor = if let Some(path) = args.file {
        Editor::open(path)?
    } else {
        Editor::new()
    };

    loop {
        terminal.draw(|frame| render(&editor, frame.area(), frame.buffer_mut()))?;

        #[allow(clippy::single_match)]
        match crossterm::event::read()? {
            Event::Key(key) => match (key.modifiers, key.code) {
                (m, KeyCode::Char('c')) if m == KeyModifiers::CONTROL => return Ok(()),
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
}

fn render(editor: &Editor, area: Rect, buffer: &mut Buffer) {
    for (line, row) in zip(
        editor.text.lines().skip(editor.vertical_scroll),
        area.rows(),
    ) {
        Text::raw(line.to_string()).render(row, buffer);
    }
}

#[derive(Default)]
struct Editor {
    text: Rope,
    vertical_scroll: usize,
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

    fn scroll_up(&mut self, distance: usize) {
        self.vertical_scroll = self.vertical_scroll.saturating_sub(distance);
    }

    fn scroll_down(&mut self, distance: usize) {
        self.vertical_scroll = min(
            self.text.line_len().saturating_sub(1),
            self.vertical_scroll + distance,
        );
    }
}
