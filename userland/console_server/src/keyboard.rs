//! Scancode decoding, ported from the kernel's src/keyboard.rs. The kernel
//! ISR (Checkpoint B/D) only acks the port and forwards the raw scancode
//! here via IPC; decoding shift state and mapping to ASCII is this task's
//! job now.

const LEFT_SHIFT: u8 = 0x2A;
const RIGHT_SHIFT: u8 = 0x36;

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

pub struct Decoder {
    shift: bool,
}

impl Decoder {
    pub const fn new() -> Self {
        Decoder { shift: false }
    }

    /// Feeds one raw scancode; returns the ASCII byte to echo, if any (key
    /// releases and non-printable/unmapped keys return `None`).
    pub fn feed(&mut self, scancode: u8) -> Option<u8> {
        let released = scancode & 0x80 != 0;
        let code = (scancode & 0x7F) as usize;

        if code == LEFT_SHIFT as usize || code == RIGHT_SHIFT as usize {
            self.shift = !released;
            return None;
        }
        if released || code >= SCANCODE_ASCII.len() {
            return None;
        }
        let ascii = if self.shift {
            SCANCODE_ASCII_SHIFTED[code]
        } else {
            SCANCODE_ASCII[code]
        };
        if ascii != 0 {
            Some(ascii)
        } else {
            None
        }
    }
}
