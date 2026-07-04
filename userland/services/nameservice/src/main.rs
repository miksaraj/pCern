//! Checkpoint H: the name-service registry. Every task gets a capability
//! to this task's public endpoint automatically (see libpcern's
//! NAMESERVICE_SLOT, wired up by loader::spawn_from_module in the
//! kernel), so it's the one piece of discovery infrastructure a task
//! doesn't need to be told about individually -- everything else (which
//! endpoint is "the console server," "the storage driver," etc.) is
//! learned dynamically through this instead of hardcoded task ids/slots.
//!
//! Registration is gated by a small compile-time allowlist mapping
//! kernel-attested task ids to the names they're allowed to claim, rather
//! than a dedicated capability kind + introspection syscall just to make
//! that one policy check possible -- main.rs's spawn order already fixes
//! which task id each trusted service gets. Lookups are open to any
//! caller.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

/// This task's own inbox -- the only capability it needs; it never looks
/// itself up, so unlike every other task it has no separate "CSlot 1 is
/// the name service" capability (main.rs only starts pointing new spawns
/// at this task's endpoint *after* it exists).
const MY_INBOX: u32 = 1;

const MAX_ENTRIES: usize = 8;

/// (kernel-attested task id, name) pairs allowed to register that name.
/// Task ids are fixed by kernel/src/main.rs's spawn order: 1 = nameservice
/// itself, 2 = console_server, 3 = storage_ata, 4 = fs_fat32, 5 = shell,
/// 6 = net_rtl8139 (spawned *last*, in both the production boot and the
/// standalone `nic_test` harness, precisely so that when no RTL8139 is
/// attached, id 6 is simply never allocated to anything else -- nothing
/// spawned after it could silently slide into this table's entry for
/// "net" the way an earlier spawn-order once let shell do). This is a
/// different crate/binary/address space from the kernel, so there's no
/// shared constant to enforce the correspondence at compile time -- but
/// kernel/src/main.rs asserts (a real `assert!`, not `debug_assert!`,
/// since this must hold in the shipped release binary) that each of
/// these tasks actually lands at the id this table expects, right after
/// spawning it, so a spawn-order change that would silently break this
/// table instead panics loudly at boot.
const ALLOWLIST: &[(u32, [u8; 8])] =
    &[(2, *b"console\0"), (3, *b"storage\0"), (4, *b"fs\0\0\0\0\0\0"), (6, *b"net\0\0\0\0\0")];

#[derive(Clone, Copy)]
struct Entry {
    name: [u8; 8],
    /// Slot, in *this task's own* CSpace, holding the capability that was
    /// registered under `name` -- reused as the transfer source for every
    /// future lookup (each lookup derives its own fresh child, so the
    /// same stored slot serves any number of lookups).
    slot: u32,
}

fn packed_name(w0: u32, w1: u32) -> [u8; 8] {
    let mut name = [0u8; 8];
    name[..4].copy_from_slice(&w0.to_le_bytes());
    name[4..].copy_from_slice(&w1.to_le_bytes());
    name
}

#[no_mangle]
#[link_section = ".text.start"]
pub extern "C" fn _start() -> ! {
    let mut registry: [Option<Entry>; MAX_ENTRIES] = [None; MAX_ENTRIES];

    loop {
        let r = libpcern::recv(MY_INBOX);
        let name = packed_name(r.w1, r.w2);

        if r.w0 == libpcern::NS_OP_REGISTER {
            let allowed = ALLOWLIST.iter().any(|(id, n)| *id == r.sender && *n == name);
            if allowed && r.transferred_slot != 0 {
                let slot_to_reuse = registry
                    .iter()
                    .position(|slot| slot.map(|entry| entry.name) == Some(name))
                    .or_else(|| registry.iter().position(|slot| slot.is_none()));
                if let Some(idx) = slot_to_reuse {
                    registry[idx] = Some(Entry { name, slot: r.transferred_slot });
                }
            }
        } else if r.w0 == libpcern::NS_OP_LOOKUP && r.transferred_slot != 0 {
            let reply_slot = r.transferred_slot;
            match registry.iter().flatten().find(|e| e.name == name) {
                Some(entry) => {
                    libpcern::send(reply_slot, 1, 0, 0, entry.slot);
                }
                None => {
                    libpcern::send(reply_slot, 0, 0, 0, 0);
                }
            }
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
