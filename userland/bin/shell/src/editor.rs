//! Phase 7, Checkpoint S: the `edit <file>` command. The actual editor
//! logic (buffer/cursor/key-application/redraw) lives in
//! `libpcern::editor::Editor`, shared with `userland/cap_test`'s
//! `editor_input_test` regression fixture so the exact code that ships
//! here is the exact code that fixture exercises. This module is just the
//! protocol wiring: load a file, hand keys to the editor, save it back.

use libpcern::editor::Editor;
use libpcern::print;

/// Runs the full-screen editor against `name` on `fs_slot`, using
/// `console_slot`/`reader_slot` for raw-mode input (the same reader
/// connection/ownership `console_slot` already established in line mode
/// -- see `console_server`'s doc comment on why raw mode reuses it rather
/// than a second connection). Loads existing content via
/// `fs_open_for_write`/`fs_read` (creating a fresh file if `name` doesn't
/// exist yet), edits interactively, saves via `fs_write` on Ctrl-S or
/// discards on Ctrl-Q, and always leaves the console back in line mode
/// before returning either way -- the caller needs no special cleanup on
/// either exit path.
///
/// `ed` is a long-lived `Editor` the caller allocated once (at shell
/// startup) and passes in fresh every time `edit` is typed, rather than a
/// new one built here -- see `libpcern::editor`'s module doc comment for
/// why: there's no way to free a `mem_alloc`'d page in this project, so
/// allocating fresh backing pages on every `edit` invocation would leak
/// them. `ed.reset()` clears its content without touching those pages.
///
/// Switches to raw mode *before* any of the setup below (resetting the
/// editor, opening/loading the file) rather than just before the key
/// loop: a user who starts typing the instant `edit <file>` completes
/// would otherwise have those keystrokes land while the connection is
/// still in line mode, where they're echoed and accumulated into the
/// line buffer instead of reaching the editor at all -- silently lost,
/// since nothing calls `CONSOLE_OP_READ_LINE` again to claim that
/// accumulation. Flipping to raw mode first means any keys typed during
/// setup are queued by console_server's `CONSOLE_OP_READ_KEY` queue
/// instead, and get applied in order once the key loop starts.
#[allow(clippy::too_many_arguments)]
pub fn run(
    console_slot: u32,
    reader_slot: u32,
    my_inbox: u32,
    fs_slot: u32,
    fs_buf_virt: u32,
    ed: &mut Editor,
    name: &[u8],
) {
    libpcern::console_set_mode(console_slot, true);

    ed.reset();

    let size = match libpcern::fs_open_for_write(fs_slot, my_inbox, name) {
        Some(s) => s,
        None => {
            libpcern::console_set_mode(console_slot, false);
            print(console_slot, b"edit: could not open/create file\n");
            return;
        }
    };

    // Load existing content a sector at a time (fs_read's usual
    // partial-transfer contract) directly into the editor's own buffer.
    let mut offset: u32 = 0;
    let mut truncated = false;
    while offset < size {
        let want = (size - offset).min(512);
        let n = libpcern::fs_read(fs_slot, my_inbox, offset, want);
        if n == 0 {
            break;
        }
        let src = unsafe { core::slice::from_raw_parts(fs_buf_virt as *const u8, n as usize) };
        if !ed.append_loaded(src) {
            truncated = true;
            break;
        }
        offset += n;
    }

    if truncated {
        print(console_slot, b"edit: warning: file exceeds 64 KiB, truncated\n");
    }

    ed.redraw(console_slot);

    let save = loop {
        let key = libpcern::console_read_key(console_slot, reader_slot);
        match ed.apply_key(key) {
            Some(save) => break save,
            None => ed.redraw(console_slot),
        }
    };

    libpcern::console_set_mode(console_slot, false);
    print(console_slot, b"\n");

    if save {
        let content = ed.content();
        let mut offset: usize = 0;
        while offset < content.len() {
            let want = (content.len() - offset).min(512);
            unsafe {
                let dst = core::slice::from_raw_parts_mut(fs_buf_virt as *mut u8, want);
                dst.copy_from_slice(&content[offset..offset + want]);
            }
            let n = libpcern::fs_write(fs_slot, my_inbox, offset as u32, want as u32);
            if n == 0 {
                print(console_slot, b"edit: FAIL (write)\n");
                return;
            }
            offset += n as usize;
        }
        // Unconditional, even when the loop above never ran (an edit that
        // deletes everything down to zero bytes has nothing to write, but
        // the file's old, longer content still needs shrinking away) --
        // this is the only thing that ever shrinks the file to match what
        // was actually just written; see FS_OP_TRUNCATE's doc comment.
        if !libpcern::fs_truncate(fs_slot, my_inbox, content.len() as u32) {
            print(console_slot, b"edit: FAIL (truncate)\n");
            return;
        }
        print(console_slot, b"edit: saved\n");
    } else {
        print(console_slot, b"edit: discarded\n");
    }
}
