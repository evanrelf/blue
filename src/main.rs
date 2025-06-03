mod defer;

use crate::defer::defer;
use camino::Utf8PathBuf;
use clap::Parser as _;
use crop::Rope;
use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::prelude::*;
use std::{fs, iter::zip};

#[derive(clap::Parser)]
struct Args {
    file: Option<Utf8PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let mut terminal = ratatui::init();
    let _guard = defer(|| ratatui::restore());

    let editor = if let Some(path) = args.file {
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
            _ => {}
        }
    }
}

fn render(editor: &Editor, area: Rect, buffer: &mut Buffer) {
    for (line, row) in zip(editor.text.lines(), area.rows()) {
        Text::raw(line.to_string()).render(row, buffer);
    }
}

#[derive(Default)]
struct Editor {
    text: Rope,
}

impl Editor {
    fn new() -> Self {
        Self::default()
    }

    fn open(path: Utf8PathBuf) -> anyhow::Result<Self> {
        let string = fs::read_to_string(path)?;
        let rope = Rope::from(string);
        Ok(Self { text: rope })
    }
}
