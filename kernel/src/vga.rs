use core::fmt;

use crate::ansi::AnsiState;
use crate::port::outb;
use crate::sync::Mutex;

pub const VGA_WIDTH: usize = 80;
pub const VGA_HEIGHT: usize = 25;
/// Physical 0xB8000 through the kernel's high-half identity alias — the low
/// identity mapping is dropped right after paging turns on (see boot.s).
const VGA_BUFFER_ADDR: usize = 0xC00B8000;

/// The 16 colors available in VGA text mode. Background colors only use the
/// low 3 bits on real hardware unless the attribute controller's blink bit
/// is reprogrammed, so bright background requests (ANSI 100-107) are mapped
/// down to their non-bright equivalent.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[allow(dead_code)] // every variant is reachable via `from_low3`/`bright`, which the analyzer can't see through
pub enum Color {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    Pink = 13,
    Yellow = 14,
    White = 15,
}

impl Color {
    pub fn from_low3(code: u8) -> Color {
        match code & 0x7 {
            0 => Color::Black,
            1 => Color::Red,
            2 => Color::Green,
            3 => Color::Brown,
            4 => Color::Blue,
            5 => Color::Magenta,
            6 => Color::Cyan,
            _ => Color::LightGray,
        }
    }

    pub fn bright(self) -> Color {
        let v = (self as u8) | 0x08;
        unsafe { core::mem::transmute(v) }
    }
}

#[derive(Clone, Copy)]
struct ScreenChar(u16);

impl ScreenChar {
    fn new(ascii: u8, fg: Color, bg: Color) -> Self {
        let attr = ((bg as u8 & 0x7) << 4) | (fg as u8 & 0x0F);
        ScreenChar(((attr as u16) << 8) | ascii as u16)
    }
}

pub struct Writer {
    col: usize,
    row: usize,
    fg: Color,
    bg: Color,
    bold: bool,
    buffer: *mut u16,
    ansi: AnsiState,
}

impl Writer {
    pub const fn new() -> Self {
        Writer {
            col: 0,
            row: 0,
            fg: Color::LightGray,
            bg: Color::Black,
            bold: false,
            buffer: VGA_BUFFER_ADDR as *mut u16,
            ansi: AnsiState::new(),
        }
    }

    fn index(row: usize, col: usize) -> usize {
        row * VGA_WIDTH + col
    }

    fn write_cell(&mut self, row: usize, col: usize, sc: ScreenChar) {
        unsafe { core::ptr::write_volatile(self.buffer.add(Self::index(row, col)), sc.0) }
    }

    fn read_cell(&self, row: usize, col: usize) -> u16 {
        unsafe { core::ptr::read_volatile(self.buffer.add(Self::index(row, col))) }
    }

    fn effective_fg(&self) -> Color {
        if self.bold {
            self.fg.bright()
        } else {
            self.fg
        }
    }

    pub fn set_fg(&mut self, c: Color) {
        self.fg = c;
    }

    pub fn set_bg(&mut self, c: Color) {
        self.bg = c;
    }

    pub fn set_bold(&mut self, bold: bool) {
        self.bold = bold;
    }

    pub fn reset_attributes(&mut self) {
        self.fg = Color::LightGray;
        self.bg = Color::Black;
        self.bold = false;
    }

    pub fn clear_screen(&mut self) {
        for row in 0..VGA_HEIGHT {
            self.clear_row(row);
        }
        self.col = 0;
        self.row = 0;
    }

    fn clear_row(&mut self, row: usize) {
        let blank = ScreenChar::new(b' ', self.effective_fg(), self.bg);
        for col in 0..VGA_WIDTH {
            self.write_cell(row, col, blank);
        }
    }

    fn clear_row_range(&mut self, row: usize, start_col: usize, end_col: usize) {
        let blank = ScreenChar::new(b' ', self.effective_fg(), self.bg);
        for col in start_col..end_col.min(VGA_WIDTH) {
            self.write_cell(row, col, blank);
        }
    }

    fn scroll(&mut self) {
        for row in 1..VGA_HEIGHT {
            for col in 0..VGA_WIDTH {
                let val = self.read_cell(row, col);
                unsafe { core::ptr::write_volatile(self.buffer.add(Self::index(row - 1, col)), val) };
            }
        }
        self.clear_row(VGA_HEIGHT - 1);
    }

    pub fn new_line(&mut self) {
        self.col = 0;
        if self.row + 1 >= VGA_HEIGHT {
            self.scroll();
        } else {
            self.row += 1;
        }
    }

    pub fn set_cursor_pos(&mut self, row: usize, col: usize) {
        self.row = row.min(VGA_HEIGHT - 1);
        self.col = col.min(VGA_WIDTH - 1);
    }

    pub fn move_cursor(&mut self, d_row: i32, d_col: i32) {
        let new_row = self.row as i32 + d_row;
        let new_col = self.col as i32 + d_col;
        self.row = new_row.clamp(0, VGA_HEIGHT as i32 - 1) as usize;
        self.col = new_col.clamp(0, VGA_WIDTH as i32 - 1) as usize;
    }

    /// ANSI Erase in Display (ED).
    pub fn erase_in_display(&mut self, mode: u16) {
        match mode {
            0 => {
                self.clear_row_range(self.row, self.col, VGA_WIDTH);
                for row in (self.row + 1)..VGA_HEIGHT {
                    self.clear_row(row);
                }
            }
            1 => {
                for row in 0..self.row {
                    self.clear_row(row);
                }
                self.clear_row_range(self.row, 0, self.col + 1);
            }
            _ => self.clear_screen(),
        }
    }

    /// ANSI Erase in Line (EL).
    pub fn erase_in_line(&mut self, mode: u16) {
        match mode {
            0 => self.clear_row_range(self.row, self.col, VGA_WIDTH),
            1 => self.clear_row_range(self.row, 0, self.col + 1),
            _ => self.clear_row_range(self.row, 0, VGA_WIDTH),
        }
    }

    pub fn put_char(&mut self, c: u8) {
        // Mirrored here (post-ANSI-parsing, so it's plain text) rather than
        // in feed_byte, purely as a debug console that never scrolls out of
        // reach the way the 80x25 VGA buffer does.
        crate::serial::write_byte(c);
        match c {
            b'\n' => self.new_line(),
            b'\r' => self.col = 0,
            0x08 => {
                if self.col > 0 {
                    self.col -= 1;
                    let blank = ScreenChar::new(b' ', self.effective_fg(), self.bg);
                    self.write_cell(self.row, self.col, blank);
                }
            }
            0x09 => {
                let next_tab = (self.col / 8 + 1) * 8;
                while self.col < next_tab && self.col < VGA_WIDTH {
                    self.put_char(b' ');
                }
            }
            byte => {
                if self.col >= VGA_WIDTH {
                    self.new_line();
                }
                let sc = ScreenChar::new(byte, self.effective_fg(), self.bg);
                self.write_cell(self.row, self.col, sc);
                self.col += 1;
            }
        }
    }

    fn feed_byte(&mut self, byte: u8) {
        let mut ansi = core::mem::replace(&mut self.ansi, AnsiState::new());
        ansi.feed(byte, self);
        self.ansi = ansi;
        self.sync_hardware_cursor();
    }

    /// Moves the blinking CRTC text cursor to match `self.row`/`self.col`.
    fn sync_hardware_cursor(&self) {
        let pos = (self.row * VGA_WIDTH + self.col) as u16;
        unsafe {
            outb(0x3D4, 0x0F);
            outb(0x3D5, (pos & 0xFF) as u8);
            outb(0x3D4, 0x0E);
            outb(0x3D5, ((pos >> 8) & 0xFF) as u8);
        }
    }
}

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            self.feed_byte(byte);
        }
        Ok(())
    }
}

pub static WRITER: Mutex<Writer> = Mutex::new(Writer::new());

pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    WRITER.lock().write_fmt(args).expect("VGA write never fails");
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::vga::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}
