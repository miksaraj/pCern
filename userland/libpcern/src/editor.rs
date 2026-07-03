//! Phase 7, Checkpoint S: a minimal full-screen text editor's core logic
//! (cursor-tracked in-memory buffer + key application + redraw), shared
//! between `userland/shell` (the production `edit <file>` command) and
//! `userland/cap_test`'s `editor_input_test` regression fixture, so the
//! exact code that ships is the exact code the fixture exercises rather
//! than a re-implementation that could quietly drift from it.
//!
//! Built on Checkpoint R's raw single-keystroke console mode. Holds a
//! file's content in one contiguous in-memory buffer, capped at
//! `EDITOR_MAX_BYTES` (16 pages = 64 KiB, built from consecutive
//! `mem_alloc` calls at consecutive virtual addresses -- each `mem_alloc`
//! mints one physical frame at a caller-chosen address, and nothing stops
//! calling it repeatedly to build a larger contiguous *virtual* range
//! with no kernel changes, unlike the physical frame allocator itself).
//! Deliberately more generous than `run`'s single-page cap for loaded
//! *code*, since editing is this phase's actual point -- a documented,
//! revisitable scope decision like that cap, not a hard ceiling: growing
//! it later is "call `mem_alloc` more times."
//!
//! Redraws the whole buffer from the top of the screen on every change
//! via ansi.rs's existing CUP (`ESC[row;colH`) and ED (`ESC[2J`) escapes,
//! already supported on the console's *output* side before this
//! checkpoint -- only raw *input* delivery (Checkpoint R) was the actual
//! gap.
//!
//! Known, deliberate scope limitation: there is no scrolling viewport --
//! content longer than the console's visible rows still prints in full
//! (scrolling the real terminal), but the cursor's absolute row/col
//! repositioning after each redraw assumes the whole buffer starts at the
//! top of the screen. Fine for this phase's target (a short file typed by
//! hand); a real scrolling viewport is future work, the same kind of
//! narrowing this project applies elsewhere (e.g. `run`'s one-page cap).

use crate::{mem_alloc, print, print_u32, KEY_DELETE, KEY_DOWN, KEY_END, KEY_HOME, KEY_LEFT, KEY_RIGHT, KEY_UP};

const EDITOR_PAGES: usize = 16;
const PAGE_SIZE: usize = 4096;
pub const EDITOR_MAX_BYTES: usize = EDITOR_PAGES * PAGE_SIZE;

/// Ctrl-S / Ctrl-Q as decoded by `keyboard::Decoder`'s Ctrl-chord
/// remapping (Ctrl-<letter> = standard ASCII control code).
pub const KEY_CTRL_S: u32 = 0x13;
pub const KEY_CTRL_Q: u32 = 0x11;
const KEY_BACKSPACE: u32 = 8;

pub struct Editor {
    buf_base: u32,
    len: usize,
    cursor: usize,
}

impl Editor {
    /// Allocates `EDITOR_PAGES` consecutive pages starting at `base`
    /// (must be page-aligned and clear of every other region the caller
    /// already uses). `None` if any allocation fails.
    pub fn new(base: u32) -> Option<Editor> {
        for i in 0..EDITOR_PAGES {
            let addr = base + (i * PAGE_SIZE) as u32;
            if mem_alloc(addr) == 0 {
                return None;
            }
        }
        Some(Editor { buf_base: base, len: 0, cursor: 0 })
    }

    fn buf(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.buf_base as *const u8, EDITOR_MAX_BYTES) }
    }

    fn buf_mut(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.buf_base as *mut u8, EDITOR_MAX_BYTES) }
    }

    pub fn content(&self) -> &[u8] {
        &self.buf()[..self.len]
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Appends `data` at the current end of the buffer (used while
    /// loading a file's existing content one sector at a time). Silently
    /// truncates at `EDITOR_MAX_BYTES` -- see the module doc comment.
    pub fn append_loaded(&mut self, data: &[u8]) {
        let room = EDITOR_MAX_BYTES - self.len;
        let n = data.len().min(room);
        let start = self.len;
        self.buf_mut()[start..start + n].copy_from_slice(&data[..n]);
        self.len += n;
    }

    fn line_start(&self, pos: usize) -> usize {
        let b = self.buf();
        let mut i = pos;
        while i > 0 && b[i - 1] != b'\n' {
            i -= 1;
        }
        i
    }

    fn line_end(&self, pos: usize) -> usize {
        let b = self.buf();
        let mut i = pos;
        while i < self.len && b[i] != b'\n' {
            i += 1;
        }
        i
    }

    fn insert_byte(&mut self, byte: u8) {
        if self.len >= EDITOR_MAX_BYTES {
            return;
        }
        let cursor = self.cursor;
        let len = self.len;
        {
            let b = self.buf_mut();
            let mut i = len;
            while i > cursor {
                b[i] = b[i - 1];
                i -= 1;
            }
            b[cursor] = byte;
        }
        self.len += 1;
        self.cursor += 1;
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let cursor = self.cursor;
        let len = self.len;
        {
            let b = self.buf_mut();
            for i in cursor..len {
                b[i - 1] = b[i];
            }
        }
        self.len -= 1;
        self.cursor -= 1;
    }

    fn delete_forward(&mut self) {
        if self.cursor >= self.len {
            return;
        }
        let cursor = self.cursor;
        let len = self.len;
        {
            let b = self.buf_mut();
            for i in (cursor + 1)..len {
                b[i - 1] = b[i];
            }
        }
        self.len -= 1;
    }

    /// Applies one decoded key (`console_server::keyboard`'s tagged `u32`
    /// scheme, mirrored by this crate's `KEY_*` constants). Returns
    /// `Some(true)` to save-and-quit (Ctrl-S), `Some(false)` to quit
    /// without saving (Ctrl-Q), `None` to keep editing.
    pub fn apply_key(&mut self, key: u32) -> Option<bool> {
        match key {
            KEY_CTRL_S => return Some(true),
            KEY_CTRL_Q => return Some(false),
            KEY_LEFT => self.cursor = self.cursor.saturating_sub(1),
            KEY_RIGHT => self.cursor = (self.cursor + 1).min(self.len),
            KEY_HOME => self.cursor = self.line_start(self.cursor),
            KEY_END => self.cursor = self.line_end(self.cursor),
            KEY_UP => {
                let cur_start = self.line_start(self.cursor);
                let col = self.cursor - cur_start;
                if cur_start > 0 {
                    let prev_end = cur_start - 1; // the '\n' ending the previous line
                    let prev_start = self.line_start(prev_end);
                    let prev_len = prev_end - prev_start;
                    self.cursor = prev_start + col.min(prev_len);
                }
            }
            KEY_DOWN => {
                let cur_start = self.line_start(self.cursor);
                let col = self.cursor - cur_start;
                let cur_end = self.line_end(self.cursor);
                if cur_end < self.len {
                    let next_start = cur_end + 1; // skip the '\n'
                    let next_end = self.line_end(next_start);
                    let next_len = next_end - next_start;
                    self.cursor = next_start + col.min(next_len);
                }
            }
            KEY_DELETE => self.delete_forward(),
            KEY_BACKSPACE => self.backspace(),
            0..=255 => {
                let byte = key as u8;
                if byte == b'\n' || byte == b'\t' || byte.is_ascii_graphic() || byte == b' ' {
                    self.insert_byte(byte);
                }
            }
            _ => {}
        }
        None
    }

    /// Clears the screen and reprints the whole buffer from the top, then
    /// repositions the cursor -- see the module doc comment for the
    /// no-scrolling-viewport limitation.
    pub fn redraw(&self, console_slot: u32) {
        print(console_slot, b"\x1b[2J\x1b[H");
        print(console_slot, self.content());

        let cur_start = self.line_start(self.cursor);
        let row = self.buf()[..cur_start].iter().filter(|&&b| b == b'\n').count();
        let col = self.cursor - cur_start;
        print(console_slot, b"\x1b[");
        print_u32(console_slot, (row + 1) as u32);
        print(console_slot, b";");
        print_u32(console_slot, (col + 1) as u32);
        print(console_slot, b"H");
    }
}
