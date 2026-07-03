# shell

The first interactive experience in pCern: type a command, something
happens. Reads a line at a time from `console_server`'s input protocol,
parses it as `<command> <argument>`, and dispatches to `fs_fat32` and the
`SYS_SPAWN_FROM_MEMORY` syscall.

## Capabilities it needs

Just the usual convention -- CSlot 1 (name service, auto-granted), CSlot 2
(its own inbox). No hardware ports or hand-wired capabilities; everything
else (`console`, `fs`) is looked up by name.

## Two endpoints, not one

`MY_INBOX` (CSlot 2) is used only for the synchronous name-service/
`fs_fat32` request/reply round trips. A **separate** endpoint
(`libpcern::endpoint_create()`) is used only for `console_server`'s
asynchronous "line ready" notifications. This isn't incidental -- it's
the exact hazard [CLAUDE.md](../../../CLAUDE.md)'s "one inbox is not
automatically safe for two roles" documents: a second typed-ahead line
completing while the shell is still blocked waiting on an `fs_open`/
`fs_read` reply would otherwise race that reply on a shared inbox.

## Commands

- **`read <file>`** -- opens `<file>` via `fs_fat32` and prints its
  contents a sector at a time, the same read loop `cap_test`'s
  `fs_client_test` already exercises.
- **`edit <file>`** -- a full-screen text editor. Opens `<file>`,
  creating a fresh zero-length one if it doesn't already exist, and
  loads any existing content into a 16-page (64 KiB) buffer (a load
  larger than that is truncated, with an explicit warning printed rather
  than silently dropping the tail). Switches the console into raw
  single-keystroke mode and redraws the buffer on every change via the
  console's existing ANSI cursor-addressing escapes. Supports
  arrow-key/Home/End/Delete/Backspace navigation and editing, plain-ASCII
  insertion, Ctrl-S to save and return to the prompt, and Ctrl-Q to
  discard and return without saving. Saving writes the buffer's full
  content via `fs_fat32`'s write support, then calls `fs_truncate` to set
  the file's size to exactly what's now in the buffer -- the only way a
  save can shrink a file (e.g. after deleting text, including deleting
  all of it), since `fs_write` on its own only ever grows or overwrites.
  The single `libpcern::editor::Editor` this command operates on is
  allocated once at shell startup and reused (`Editor::reset()`) for
  every `edit` invocation afterward, rather than allocated fresh each
  time -- there's no syscall to free a `mem_alloc`'d page in this
  project, so a fresh allocation per invocation would leak it. The
  editor's actual logic lives in `libpcern::editor::Editor` -- see that
  crate's README for why, and
  `userland/cap_test/src/bin/editor_input_test.rs` for the regression
  fixture that exercises the exact same code.
- **`run <file>`** -- opens `<file>`, reads it into a single
  `mem_alloc`'d page (capped at 4096 bytes -- see below), and spawns it
  via `SYS_SPAWN_FROM_MEMORY`. Prints the new task's id, or an error if
  the file doesn't exist, is too large, or the spawn itself fails.
- **`help`** -- lists the commands above.

Anything else is echoed back as "unknown command".

## Why `run` is capped at one page

`fs_fat32`'s `fs_read` only ever returns up to one 512-byte sector per
call, always written to the *start* of the shared buffer regardless of
file offset (see `fs_fat32`'s own `read_file`) -- so loading a program
bigger than one sector means copying each chunk out to the correct offset
in a second, separate page before handing that page to
`spawn_from_memory`. This shell does exactly that, up to one page (4096
bytes, the same cap every `MemoryGrant` already has) rather than adding
multi-page assembly for what's still just running a few small programs.
A program's frames and page directory are never reclaimed once it exits,
same as every task spawned any other way today -- a known, deliberate gap
in this project's current scope, not something specific to this shell.
