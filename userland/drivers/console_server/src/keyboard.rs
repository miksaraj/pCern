//! Scancode decoding, ported from the kernel's src/keyboard.rs. The kernel
//! ISR (Checkpoint B/D) only acks the port and forwards the raw scancode
//! here via IPC; decoding shift state and mapping to ASCII is this task's
//! job now.
//!
//! Phase 7, Checkpoint R adds Ctrl-state tracking (mirroring the existing
//! Shift tracking exactly -- one more make/break scancode pair) and
//! 0xE0-prefixed extended-key decoding (arrows, Home/End/Delete/PageUp/
//! PageDown), needed for the new raw-keystroke console input mode and the
//! full-screen editor built on it. `feed` now returns a tagged `u32`
//! instead of `Option<u8>`: `0..=255` is a plain ASCII byte (unchanged
//! meaning for every existing line-mode caller), `>= 256` is one of the
//! `KEY_*` constants below, which have no ASCII representation. A
//! Ctrl-chord on a letter reuses the existing SCANCODE_ASCII lookup
//! rather than a second table: if Ctrl is held and the resolved ASCII
//! byte is a letter, it's remapped to the standard ASCII control code
//! (Ctrl-A=0x01 .. Ctrl-Z=0x1A) instead of being returned as-is.

const LEFT_SHIFT: u8 = 0x2A;
const RIGHT_SHIFT: u8 = 0x36;
const LEFT_CTRL: u8 = 0x1D;
/// Scancode set 1's prefix byte for the extended (E0) key set -- arrives
/// as its own byte before the actual make/break code, so decoding it
/// spans two `feed()` calls (see `pending_extended`).
const EXTENDED_PREFIX: u8 = 0xE0;

// PS/2 scancode set 1, unshifted/shifted ASCII for the make codes we handle.
const SCANCODE_ASCII: [u8; 58] = [
    0, 27, b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'0', b'-', b'=', 8, b'\t', b'q',
    b'w', b'e', b'r', b't', b'y', b'u', b'i', b'o', b'p', b'[', b']', b'\n', 0, b'a', b's', b'd',
    b'f', b'g', b'h', b'j', b'k', b'l', b';', b'\'', b'`', 0, b'\\', b'z', b'x', b'c', b'v', b'b',
    b'n', b'm', b',', b'.', b'/', 0, b'*', 0, b' ',
];

const SCANCODE_ASCII_SHIFTED: [u8; 58] = [
    0, 27, b'!', b'@', b'#', b'$', b'%', b'^', b'&', b'*', b'(', b')', b'_', b'+', 8, b'\t', b'Q',
    b'W', b'E', b'R', b'T', b'Y', b'U', b'I', b'O', b'P', b'{', b'}', b'\n', 0, b'A', b'S', b'D',
    b'F', b'G', b'H', b'J', b'K', b'L', b':', b'"', b'~', 0, b'|', b'Z', b'X', b'C', b'V', b'B',
    b'N', b'M', b'<', b'>', b'?', 0, b'*', 0, b' ',
];

/// Tagged non-ASCII key values `feed` can return -- deliberately starting
/// well above any byte value so `0..=255` unambiguously means "plain
/// ASCII" to every caller.
pub const KEY_UP: u32 = 256;
pub const KEY_DOWN: u32 = 257;
pub const KEY_LEFT: u32 = 258;
pub const KEY_RIGHT: u32 = 259;
pub const KEY_HOME: u32 = 260;
pub const KEY_END: u32 = 261;
pub const KEY_DELETE: u32 = 262;
pub const KEY_PAGE_UP: u32 = 263;
pub const KEY_PAGE_DOWN: u32 = 264;

fn extended_key(code: u8) -> Option<u32> {
    match code {
        0x48 => Some(KEY_UP),
        0x50 => Some(KEY_DOWN),
        0x4B => Some(KEY_LEFT),
        0x4D => Some(KEY_RIGHT),
        0x47 => Some(KEY_HOME),
        0x4F => Some(KEY_END),
        0x53 => Some(KEY_DELETE),
        0x49 => Some(KEY_PAGE_UP),
        0x51 => Some(KEY_PAGE_DOWN),
        _ => None,
    }
}

pub struct Decoder {
    shift: bool,
    ctrl: bool,
    /// Set on seeing a bare `0xE0` byte, consumed on the very next byte
    /// fed in (which is the extended set's actual make/break code).
    pending_extended: bool,
}

impl Decoder {
    pub const fn new() -> Self {
        Decoder { shift: false, ctrl: false, pending_extended: false }
    }

    /// Feeds one raw scancode; returns the decoded key, if any (key
    /// releases and non-printable/unmapped keys return `None`). See the
    /// module doc comment for the `u32` tagging scheme.
    pub fn feed(&mut self, scancode: u8) -> Option<u32> {
        if scancode == EXTENDED_PREFIX {
            self.pending_extended = true;
            return None;
        }

        let released = scancode & 0x80 != 0;
        let code = scancode & 0x7F;

        if self.pending_extended {
            self.pending_extended = false;
            return if released { None } else { extended_key(code) };
        }

        if code == LEFT_SHIFT || code == RIGHT_SHIFT {
            self.shift = !released;
            return None;
        }
        if code == LEFT_CTRL {
            self.ctrl = !released;
            return None;
        }
        if released || code as usize >= SCANCODE_ASCII.len() {
            return None;
        }
        let ascii = if self.shift {
            SCANCODE_ASCII_SHIFTED[code as usize]
        } else {
            SCANCODE_ASCII[code as usize]
        };
        if ascii == 0 {
            return None;
        }
        if self.ctrl && ascii.is_ascii_alphabetic() {
            return Some((ascii.to_ascii_uppercase() - b'A' + 1) as u32);
        }
        Some(ascii as u32)
    }
}
