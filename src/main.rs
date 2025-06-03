mod defer;

use crate::defer::defer;
use camino::Utf8PathBuf;
use clap::Parser as _;
use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::{text::Text, widgets::Widget as _};

#[derive(clap::Parser)]
struct Args {
    file: Option<Utf8PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let _args = Args::parse();

    let mut terminal = ratatui::init();
    let _guard = defer(|| ratatui::restore());

    loop {
        terminal
            .draw(|frame| Text::raw("Hello, world!").render(frame.area(), frame.buffer_mut()))?;

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
