mod defer;

use crate::defer::defer;
use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::{text::Text, widgets::Widget as _};

fn main() -> anyhow::Result<()> {
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
