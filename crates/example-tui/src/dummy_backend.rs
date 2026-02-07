use std::{fmt::Display, io::Write};

use anes::{ResetAttributes, SetAttribute, SetBackgroundColor, SetForegroundColor};
use ratatui::{
    backend::WindowSize,
    layout::{Position, Size},
    style::{Color, Modifier},
};
use std::io::Result as IOResult;

/// A pure ANSI implementation of RataTUI's backend.
///
/// The caller must provide a callback for fetching window size, and writing output to stdout.
pub struct AnsiBackend<T: Write> {
    size: Size,
    pos: Option<Position>,
    writer: T,
}

impl<T: Write> AnsiBackend<T> {
    pub fn new(size: Size, writer: T) -> Self {
        Self {
            size,
            writer,
            pos: None,
        }
    }
}

impl<T: Write> ratatui::backend::Backend for AnsiBackend<T> {
    fn draw<'a, I>(&mut self, content: I) -> IOResult<()>
    where
        I: Iterator<Item = (u16, u16, &'a ratatui::buffer::Cell)>,
    {
        let mut prev_pos: Option<Position> = None;
        let mut prev_mod: Option<Modifier> = None;
        let mut prev_fg: Option<Color> = None;
        let mut prev_bg: Option<Color> = None;

        for (x, y, cell) in content {
            if cell.skip {
                continue;
            }

            let mut new_pos = Position { x, y };
            if let Some(prev_pos) = prev_pos {
                if new_pos != prev_pos {
                    self.set_cursor_position(new_pos)?;
                }
            } else {
                self.set_cursor_position(new_pos)?;
            }
            prev_pos = Some(new_pos);

            self.apply_modifiers(&mut prev_mod, &cell.modifier)?;

            // TODO: colors
            if prev_bg != Some(cell.bg) {
                self.push(SetBackgroundColor(ansi_color(cell.bg)))?;
                prev_bg = Some(cell.bg);
            }

            if prev_fg != Some(cell.fg) {
                self.push(SetForegroundColor(ansi_color(cell.fg)))?;
                prev_fg = Some(cell.fg);
            }

            self.writer.write(cell.symbol().as_bytes())?;
            // TODO: Check unicode width. (Why doesn't Ratatui give us this?)
            let width = 1;
            new_pos.x += width;
            // TODO: Wrap?
            prev_pos = Some(new_pos)
        }

        if let Some(pos) = prev_pos {
            self.pos = Some(pos);
        }

        Ok(())
    }

    fn hide_cursor(&mut self) -> IOResult<()> {
        write!(self.writer, "{}", anes::HideCursor)
    }

    fn show_cursor(&mut self) -> IOResult<()> {
        write!(self.writer, "{}", anes::ShowCursor)
    }

    fn get_cursor_position(&mut self) -> IOResult<ratatui::prelude::Position> {
        let pos = match self.pos {
            Some(pos) => pos,
            None => {
                let new_pos = Position { x: 0, y: 0 };
                self.set_cursor_position(new_pos)?;
                self.pos = Some(new_pos);
                new_pos
            }
        };

        Ok(pos)
    }

    fn set_cursor_position<P: Into<ratatui::prelude::Position>>(
        &mut self,
        new_pos_into: P,
    ) -> IOResult<()> {
        let new_pos: Position = new_pos_into.into();
        if Some(new_pos) == self.pos {
            return Ok(());
        }

        self.push(anes::MoveCursorTo(new_pos.x + 1, new_pos.y + 1))?;
        self.pos = Some(new_pos);
        Ok(())
    }

    fn clear(&mut self) -> IOResult<()> {
        // If there's a remaining color it'll set the whole screen to that color. We don't want that:
        self.push(anes::ResetAttributes)?;
        self.push(anes::ClearBuffer::All)
    }

    fn size(&self) -> IOResult<ratatui::prelude::Size> {
        let size = self.size;
        Ok(size.into())
    }

    fn window_size(&mut self) -> IOResult<ratatui::backend::WindowSize> {
        Ok(WindowSize {
            columns_rows: self.size()?,
            pixels: Default::default(),
        })
    }

    fn flush(&mut self) -> IOResult<()> {
        self.writer.flush()
    }
}

impl<T> AnsiBackend<T>
where
    T: Write,
{
    fn apply_modifiers(&mut self, old: &mut Option<Modifier>, new: &Modifier) -> IOResult<()> {
        if let Some(prev) = old {
            if prev == new {
                return Ok(());
            }
        }

        let prev = match old {
            Some(prev) => prev.clone(),
            None => {
                // We don't know what the previous state was, so reset it to be safe:
                self.push(ResetAttributes)?;
                Modifier::empty()
            }
        };

        let to_set = *new - prev;
        let to_del = prev - *new;

        use SetAttribute as Set;
        use anes::Attribute as AA;

        // Bold/faint/normal are all mutually exclusive
        if to_set.contains(Modifier::BOLD) {
            self.push(Set(AA::Bold))?;
        } else if to_set.contains(Modifier::DIM) {
            self.push(Set(AA::Faint))?;
        } else if to_del.contains(Modifier::DIM) || to_del.contains(Modifier::BOLD) {
            self.push(Set(AA::Normal))?;
        }

        if to_set.contains(Modifier::CROSSED_OUT) {
            self.push(Set(AA::Crossed))?;
        } else if to_del.contains(Modifier::CROSSED_OUT) {
            self.push(Set(AA::CrossedOff))?;
        }

        if to_set.contains(Modifier::HIDDEN) {
            self.push(Set(AA::Conceal))?;
        } else if to_del.contains(Modifier::HIDDEN) {
            self.push(Set(AA::ConcealOff))?;
        }

        if to_set.contains(Modifier::ITALIC) {
            self.push(Set(AA::Italic))?;
        } else if to_del.contains(Modifier::ITALIC) {
            self.push(Set(AA::ItalicOff))?;
        }

        if to_set.contains(Modifier::RAPID_BLINK) || to_set.contains(Modifier::SLOW_BLINK) {
            self.push(Set(AA::Blink))?;
        } else if to_del.contains(Modifier::RAPID_BLINK) || to_del.contains(Modifier::SLOW_BLINK) {
            self.push(Set(AA::BlinkOff))?;
        }

        if to_set.contains(Modifier::REVERSED) {
            self.push(Set(AA::Reverse))?;
        } else if to_del.contains(Modifier::REVERSED) {
            self.push(Set(AA::ReverseOff))?;
        }

        if to_set.contains(Modifier::UNDERLINED) {
            self.push(Set(AA::Underline))?;
        } else if to_del.contains(Modifier::UNDERLINED) {
            self.push(Set(AA::UnderlineOff))?;
        }

        *old = Some(*new);
        Ok(())
    }

    fn push(&mut self, ansi: impl Display) -> IOResult<()> {
        write!(self.writer, "{}", ansi)
    }
}

fn ansi_color(color: ratatui::style::Color) -> anes::Color {
    use anes::Color as AColor;
    use ratatui::style::Color as RColor;

    match color {
        RColor::Reset => AColor::Default,
        RColor::Black => AColor::Black,
        RColor::Red => AColor::DarkRed,
        RColor::Green => AColor::DarkGreen,
        RColor::Yellow => AColor::DarkYellow,
        RColor::Blue => AColor::DarkBlue,
        RColor::Magenta => AColor::DarkMagenta,
        RColor::Cyan => AColor::DarkCyan,
        RColor::Gray => AColor::DarkGray,
        RColor::DarkGray => AColor::DarkGray,
        RColor::LightRed => AColor::Red,
        RColor::LightGreen => AColor::Green,
        RColor::LightYellow => AColor::Yellow,
        RColor::LightBlue => AColor::Blue,
        RColor::LightMagenta => AColor::Magenta,
        RColor::LightCyan => AColor::Cyan,
        RColor::White => AColor::White,
        RColor::Rgb(r, g, b) => AColor::Rgb(r, g, b),
        RColor::Indexed(code) => AColor::Ansi(code),
    }
}
