use crate::{
    display_width::DisplayWidth as _,
    graphemes::{ceil_grapheme_boundary, next_grapheme_boundary, prev_grapheme_boundary},
};
use camino::{Utf8Path, Utf8PathBuf};
use clap::Parser as _;
use crop::Rope;
use std::{
    cmp::{max, min},
    fs, iter, mem,
    process::ExitCode,
};

pub struct Editor {
    pub pwd: Option<Utf8PathBuf>,
    pub path: Option<Utf8PathBuf>,
    pub modified: bool,
    pub text: Rope,
    pub anchor: usize,
    pub head: usize,
    desired_column: usize,
    pub vertical_scroll: usize,
    pub mode: Mode,
    pub command: Rope,
    pub command_cursor: usize,
    pub message: Option<Result<String, String>>,
    pub exit_code: Option<ExitCode>,
}

impl Editor {
    pub fn new() -> anyhow::Result<Self> {
        Self::try_from(Rope::new())
    }

    pub fn open(path: impl AsRef<Utf8Path>) -> anyhow::Result<Self> {
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

    pub fn save(&mut self) -> anyhow::Result<()> {
        if let Some(path) = &self.path {
            let bytes = self.text.bytes().collect::<Vec<_>>();
            fs::write(path, bytes)?;
            self.modified = false;
        }
        Ok(())
    }

    fn update_desired_column(&mut self) {
        let current_line_index = self.text.line_of_byte(self.head);
        let current_line_byte_index = self.text.byte_of_line(current_line_index);
        self.desired_column = self
            .text
            .byte_slice(current_line_byte_index..self.head)
            .display_width();
    }

    pub fn extend_to(&mut self, byte_offset: usize) {
        debug_assert!(self.text.is_grapheme_boundary(byte_offset));
        if self.is_backward() {
            self.head = byte_offset;
        } else {
            self.head = ceil_grapheme_boundary(&self.text.byte_slice(..), byte_offset + 1);
        }
        self.update_desired_column();
    }

    pub fn extend_left(&mut self, count: usize) {
        debug_assert!(self.text.is_grapheme_boundary(self.head));
        for _ in 0..count {
            match prev_grapheme_boundary(&self.text.byte_slice(..), self.head) {
                Some(prev) if self.head != prev => self.head = prev,
                _ => break,
            }
        }
        self.update_desired_column();
    }

    pub fn extend_right(&mut self, count: usize) {
        debug_assert!(self.text.is_grapheme_boundary(self.head));
        for _ in 0..count {
            match next_grapheme_boundary(&self.text.byte_slice(..), self.head) {
                Some(next) if self.head != next => self.head = next,
                _ => break,
            }
        }
        self.update_desired_column();
    }

    pub fn extend_up(&mut self, count: usize) {
        debug_assert!(self.text.is_grapheme_boundary(self.head));
        for _ in 0..count {
            let current_line_index = self.text.line_of_byte(self.head);
            if current_line_index == 0 {
                break;
            }
            let target_line_index = current_line_index - 1;
            let target_line_byte_index = self.text.byte_of_line(target_line_index);
            let target_line_slice = self.text.line(target_line_index);
            let mut target_line_prefix = 0;
            let mut byte_offset = target_line_byte_index;
            for grapheme in target_line_slice.graphemes() {
                let grapheme_width = grapheme.as_ref().display_width();
                if target_line_prefix + grapheme_width > self.desired_column {
                    break;
                }
                target_line_prefix += grapheme_width;
                byte_offset += grapheme.len();
            }
            self.head = byte_offset;
        }
    }

    pub fn extend_down(&mut self, count: usize) {
        debug_assert!(self.text.is_grapheme_boundary(self.head));
        for _ in 0..count {
            let current_line_index = self.text.line_of_byte(self.head);
            let target_line_index = current_line_index + 1;
            if target_line_index >= self.text.line_len() {
                self.head = self.text.byte_len();
                break;
            }
            let target_line_byte_index = self.text.byte_of_line(target_line_index);
            let target_line_slice = self.text.line(target_line_index);
            let mut target_line_prefix = 0;
            let mut byte_offset = target_line_byte_index;
            for grapheme in target_line_slice.graphemes() {
                let grapheme_width = grapheme.as_ref().display_width();
                if target_line_prefix + grapheme_width > self.desired_column {
                    break;
                }
                target_line_prefix += grapheme_width;
                byte_offset += grapheme.len();
            }
            self.head = byte_offset;
        }
    }

    pub fn extend_line_start(&mut self) {
        debug_assert!(self.text.is_grapheme_boundary(self.head));
        let line_index = self.text.line_of_byte(self.head);
        let line_start_byte_index = self.text.byte_of_line(line_index);
        self.head = line_start_byte_index;
        if self.is_forward() {
            self.extend_right(1);
        }
        self.update_desired_column();
    }

    pub fn extend_line_end(&mut self) {
        debug_assert!(self.text.is_grapheme_boundary(self.head));
        if self.head >= self.text.byte_len() {
            return;
        }
        let line_index = self.text.line_of_byte(self.head);
        let line_start_byte_index = self.text.byte_of_line(line_index);
        let line = self.text.line(line_index);
        let line_end_byte_index = line_start_byte_index + line.byte_len();
        self.head = line_end_byte_index;
        if self.is_backward() {
            self.extend_left(1);
        }
        self.update_desired_column();
    }

    pub fn move_to(&mut self, byte_offset: usize) {
        self.extend_to(byte_offset);
        self.reduce();
        self.extend_left(1);
        self.flip_forward();
    }

    pub fn move_left(&mut self, count: usize) {
        self.extend_left(count);
        self.reduce();
    }

    pub fn move_right(&mut self, count: usize) {
        self.extend_right(count);
        self.reduce();
    }

    pub fn move_up(&mut self, count: usize) {
        self.extend_up(count);
        self.reduce();
    }

    pub fn move_down(&mut self, count: usize) {
        self.extend_down(count);
        self.reduce();
    }

    pub fn move_line_start(&mut self) {
        self.extend_line_start();
        self.reduce();
    }

    pub fn move_line_end(&mut self) {
        self.extend_line_end();
        self.reduce();
    }

    pub fn command_mode_move_left(&mut self, count: usize) {
        debug_assert!(self.mode == Mode::Command);
        debug_assert!(self.command.is_grapheme_boundary(self.command_cursor));
        for _ in 0..count {
            match prev_grapheme_boundary(&self.command.byte_slice(..), self.command_cursor) {
                Some(prev) if self.command_cursor != prev => self.command_cursor = prev,
                _ => break,
            }
        }
    }

    pub fn command_mode_move_right(&mut self, count: usize) {
        debug_assert!(self.mode == Mode::Command);
        debug_assert!(self.command.is_grapheme_boundary(self.command_cursor));
        for _ in 0..count {
            match next_grapheme_boundary(&self.command.byte_slice(..), self.command_cursor) {
                Some(next) if self.command_cursor != next => self.command_cursor = next,
                _ => break,
            }
        }
    }

    pub fn command_mode_delete_before(&mut self) {
        debug_assert!(self.mode == Mode::Command);
        debug_assert!(self.command.is_grapheme_boundary(self.command_cursor));
        if self.command_cursor > 0 {
            self.command.delete(0..self.command_cursor);
            self.command_cursor = 0;
        }
    }

    pub fn command_mode_delete_after(&mut self) {
        debug_assert!(self.mode == Mode::Command);
        debug_assert!(self.command.is_grapheme_boundary(self.command_cursor));
        let end = self.command.byte_len();
        if self.command_cursor < end {
            self.command.delete(self.command_cursor..end);
        }
    }

    pub fn is_forward(&self) -> bool {
        self.anchor <= self.head
    }

    pub fn is_backward(&self) -> bool {
        !self.is_forward()
    }

    pub fn flip(&mut self) {
        mem::swap(&mut self.anchor, &mut self.head);
    }

    pub fn flip_forward(&mut self) {
        if !self.is_forward() {
            self.flip();
        }
    }

    pub fn reduce(&mut self) {
        self.anchor = self.head;
    }

    pub fn scroll_up(&mut self, distance: usize) {
        debug_assert!(self.vertical_scroll < self.text.line_len());
        self.vertical_scroll = self.vertical_scroll.saturating_sub(distance);
    }

    pub fn scroll_down(&mut self, distance: usize) {
        debug_assert!(self.vertical_scroll < self.text.line_len());
        self.vertical_scroll = min(
            self.text.line_len().saturating_sub(1),
            self.vertical_scroll + distance,
        );
    }

    pub fn insert(&mut self, text: &str) {
        self.text.insert(self.head, text);
        self.head += text.len();
        self.update_desired_column();
        self.reduce();
        self.modified = true;
    }

    pub fn delete_before(&mut self) {
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

    pub fn delete(&mut self) {
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
    pub fn delete_after(&mut self) {
        if let Some(grapheme) = self.text.byte_slice(self.head..).graphemes().next() {
            let start = self.head;
            let end = start + grapheme.len();
            self.text.delete(start..end);
            self.modified = true;
            debug_assert!(self.text.is_grapheme_boundary(self.anchor));
            debug_assert!(self.text.is_grapheme_boundary(self.head));
        }
    }

    pub fn execute_command(&mut self) -> anyhow::Result<()> {
        #[derive(clap::Parser)]
        #[clap(
            disable_help_flag = true,
            disable_help_subcommand = true,
            override_usage = ""
        )]
        enum Command {
            Echo {
                #[clap(long)]
                error: bool,
                message: Vec<String>,
            },
            #[clap(alias = "w")]
            Write,
            #[clap(alias = "q")]
            Quit { exit_code: Option<u8> },
            #[clap(name = "quit!", alias = "q!")]
            QuitForce { exit_code: Option<u8> },
            #[clap(name = "write-quit", alias = "wq")]
            WriteQuit { exit_code: Option<u8> },
        }
        let Ok(args) = shellwords::split(&self.command.to_string()) else {
            self.message = Some(Err(String::from("Invalid command")));
            self.command = Rope::new();
            self.command_cursor = 0;
            self.mode = Mode::Normal;
            return Ok(());
        };
        if args.is_empty() {
            self.command = Rope::new();
            self.command_cursor = 0;
            self.mode = Mode::Normal;
            return Ok(());
        }
        let args = iter::once(String::from("blue")).chain(args);
        let command = match Command::try_parse_from(args) {
            Ok(command) => command,
            Err(error) => {
                let error = error.to_string();
                let error = error
                    .strip_prefix("error: ")
                    .unwrap_or(&error)
                    .lines()
                    .next()
                    .unwrap_or("");
                self.message = Some(Err(error.to_string()));
                self.command = Rope::new();
                self.command_cursor = 0;
                self.mode = Mode::Normal;
                return Ok(());
            }
        };
        match command {
            Command::Echo { error, message } => {
                if error {
                    self.message = Some(Err(message.join(" ")));
                } else {
                    self.message = Some(Ok(message.join(" ")));
                }
            }
            Command::Write => {
                self.save()?;
            }
            Command::Quit { exit_code } => {
                if self.modified {
                    self.message = Some(Err(String::from("Unsaved changes")));
                } else {
                    self.exit_code = if let Some(exit_code) = exit_code {
                        Some(ExitCode::from(exit_code))
                    } else {
                        Some(ExitCode::SUCCESS)
                    };
                }
            }
            Command::QuitForce { exit_code } => {
                self.exit_code = if let Some(exit_code) = exit_code {
                    Some(ExitCode::from(exit_code))
                } else {
                    Some(ExitCode::SUCCESS)
                };
            }
            Command::WriteQuit { exit_code } => {
                self.save()?;
                self.exit_code = if let Some(exit_code) = exit_code {
                    Some(ExitCode::from(exit_code))
                } else {
                    Some(ExitCode::SUCCESS)
                };
            }
        }
        self.command = Rope::new();
        self.command_cursor = 0;
        self.mode = Mode::Normal;
        Ok(())
    }
}

impl TryFrom<Rope> for Editor {
    type Error = anyhow::Error;
    fn try_from(rope: Rope) -> Result<Self, Self::Error> {
        Ok(Self {
            pwd: None,
            path: None,
            modified: false,
            text: rope,
            anchor: 0,
            head: 0,
            desired_column: 0,
            vertical_scroll: 0,
            mode: Mode::Normal,
            command: Rope::new(),
            command_cursor: 0,
            message: None,
            exit_code: None,
        })
    }
}

#[derive(PartialEq)]
pub enum Mode {
    Normal,
    Goto,
    Insert,
    Command,
}
