//! Ported from the kernel's src/ansi.rs (Checkpoint D moves ANSI parsing +
//! VGA writing out of the kernel and into this userspace console server;
//! the state machine itself is unchanged).

use crate::vga::{Color, Writer};

const MAX_PARAMS: usize = 8;

#[derive(Clone, Copy)]
enum State {
    Ground,
    Escape,
    Csi,
}

/// Minimal ANSI/VT100 escape sequence parser feeding directly into a
/// [`Writer`]. Supports cursor movement (CUU/CUD/CUF/CUB/CUP), erase
/// (ED/EL) and SGR (colors + bold) — enough for coloured shells and simple
/// full-screen terminal programs.
pub struct AnsiState {
    state: State,
    params: [u16; MAX_PARAMS],
    nparams: usize,
}

impl AnsiState {
    pub const fn new() -> Self {
        AnsiState {
            state: State::Ground,
            params: [0; MAX_PARAMS],
            nparams: 0,
        }
    }

    pub fn feed(&mut self, byte: u8, w: &mut Writer) {
        match self.state {
            State::Ground => {
                if byte == 0x1B {
                    self.state = State::Escape;
                } else {
                    w.put_char(byte);
                }
            }
            State::Escape => match byte {
                b'[' => {
                    self.params = [0; MAX_PARAMS];
                    self.nparams = 0;
                    self.state = State::Csi;
                }
                _ => self.state = State::Ground,
            },
            State::Csi => match byte {
                b'0'..=b'9' => {
                    if self.nparams == 0 {
                        self.nparams = 1;
                    }
                    let idx = self.nparams - 1;
                    if idx < MAX_PARAMS {
                        self.params[idx] = self.params[idx]
                            .saturating_mul(10)
                            .saturating_add((byte - b'0') as u16);
                    }
                }
                b';' => {
                    if self.nparams < MAX_PARAMS {
                        self.nparams += 1;
                    }
                }
                b'?' => {
                    // Private-mode sequences (e.g. DEC modes) are accepted but ignored.
                }
                0x40..=0x7E => {
                    self.execute(byte, w);
                    self.state = State::Ground;
                }
                _ => self.state = State::Ground,
            },
        }
    }

    fn param(&self, idx: usize, default: u16) -> u16 {
        if idx < self.nparams && self.params[idx] != 0 {
            self.params[idx]
        } else {
            default
        }
    }

    fn execute(&mut self, byte: u8, w: &mut Writer) {
        match byte {
            b'A' => w.move_cursor(-(self.param(0, 1) as i32), 0),
            b'B' => w.move_cursor(self.param(0, 1) as i32, 0),
            b'C' => w.move_cursor(0, self.param(0, 1) as i32),
            b'D' => w.move_cursor(0, -(self.param(0, 1) as i32)),
            b'H' | b'f' => {
                let row = self.param(0, 1).saturating_sub(1) as usize;
                let col = self.param(1, 1).saturating_sub(1) as usize;
                w.set_cursor_pos(row, col);
            }
            b'J' => w.erase_in_display(self.raw_param(0)),
            b'K' => w.erase_in_line(self.raw_param(0)),
            b'm' => self.apply_sgr(w),
            _ => {}
        }
    }

    fn raw_param(&self, idx: usize) -> u16 {
        if idx < self.nparams {
            self.params[idx]
        } else {
            0
        }
    }

    fn apply_sgr(&self, w: &mut Writer) {
        if self.nparams == 0 {
            w.reset_attributes();
            return;
        }
        for i in 0..self.nparams {
            match self.params[i] {
                0 => w.reset_attributes(),
                1 => w.set_bold(true),
                22 => w.set_bold(false),
                30..=37 => w.set_fg(Color::from_low3((self.params[i] - 30) as u8)),
                39 => w.set_fg(Color::LightGray),
                40..=47 => w.set_bg(Color::from_low3((self.params[i] - 40) as u8)),
                49 => w.set_bg(Color::Black),
                90..=97 => w.set_fg(Color::from_low3((self.params[i] - 90) as u8).bright()),
                100..=107 => w.set_bg(Color::from_low3((self.params[i] - 100) as u8)),
                _ => {}
            }
        }
    }
}
