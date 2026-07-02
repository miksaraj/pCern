//! Checkpoint J: a read-only FAT32 filesystem server. Looks up "storage"
//! via the name service and reads sectors through it (Checkpoint I),
//! parses just enough of the BPB/FAT/root-directory structures to open a
//! root-directory 8.3-named file and read it back, then registers as
//! "fs" and serves the same kind of shared-memory-grant protocol to its
//! own clients that storage_ata serves to it (see libpcern's FS_OP_*/
//! fs_connect/fs_open/fs_read).
//!
//! Scope for v1 (deliberately narrow, matching this phase's other
//! scope-narrowing calls): root-directory files only, no subdirectory
//! traversal, 8.3 names only (long-filename and volume-label entries are
//! skipped, never matched), read-only, one client and one open file at a
//! time. The one FAT32-specific landmine worth calling out: the root
//! directory is itself an ordinary cluster chain rooted at `root_cluster`
//! (not a fixed region the way FAT16's root dir is), so it's walked with
//! exactly the same `next_cluster` logic as file data.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

/// CSlot 1 is the name service (auto-granted); this is this task's own
/// inbox, used to serve *this task's own* fs clients. Kept separate from
/// the endpoint used to talk to storage (see `STORAGE_REPLY` below) so
/// the two roles (fs server / storage client) never contend over which
/// message a given `recv` is supposed to be.
const MY_INBOX: u32 = 2;

/// Where the storage service's shared sector buffer gets mapped in this
/// task's own address space.
const STORAGE_BUF_VIRT: u32 = 0x0080_0000;
/// Where a connected fs client's shared buffer gets mapped -- a
/// different address than `STORAGE_BUF_VIRT` since both are mapped
/// simultaneously in this same address space.
const CLIENT_BUF_VIRT: u32 = 0x0090_0000;

const SECTOR_SIZE: usize = 512;
const DIRENT_SIZE: usize = 32;
const FAT32_EOC_MIN: u32 = 0x0FFF_FFF8;

struct Bpb {
    sectors_per_cluster: u32,
    first_data_sector: u32,
    fat_begin_sector: u32,
    root_cluster: u32,
}

impl Bpb {
    fn cluster_to_sector(&self, cluster: u32) -> u32 {
        self.first_data_sector + (cluster - 2) * self.sectors_per_cluster
    }

    fn cluster_size_bytes(&self) -> u32 {
        self.sectors_per_cluster * SECTOR_SIZE as u32
    }
}

struct OpenFile {
    start_cluster: u32,
    size: u32,
}

fn storage_buf() -> &'static mut [u8; SECTOR_SIZE] {
    unsafe { &mut *(STORAGE_BUF_VIRT as *mut [u8; SECTOR_SIZE]) }
}

fn client_buf() -> &'static mut [u8] {
    unsafe { core::slice::from_raw_parts_mut(CLIENT_BUF_VIRT as *mut u8, SECTOR_SIZE) }
}

fn read_sector(storage_slot: u32, storage_reply: u32, lba: u32) -> bool {
    libpcern::storage_read_block(storage_slot, storage_reply, lba)
}

/// Returns `None` if LBA 0 can't be read at all (no disk attached behind
/// storage_ata -- this task stays alive but never registers "fs" rather
/// than treating that as fatal, the same graceful-idle behavior
/// storage_ata itself already has when nobody's asked it to do anything)
/// or doesn't look like a valid FAT32 BPB.
fn parse_bpb(storage_slot: u32, storage_reply: u32) -> Option<Bpb> {
    if !read_sector(storage_slot, storage_reply, 0) {
        return None;
    }
    let b = storage_buf();
    let bytes_per_sector = u16::from_le_bytes([b[11], b[12]]) as u32;
    let sectors_per_cluster = b[13] as u32;
    let reserved_sector_count = u16::from_le_bytes([b[14], b[15]]) as u32;
    let num_fats = b[16] as u32;
    let fat_sz32 = u32::from_le_bytes([b[36], b[37], b[38], b[39]]);
    let root_cluster = u32::from_le_bytes([b[44], b[45], b[46], b[47]]);
    let signature = [b[510], b[511]];

    if bytes_per_sector != SECTOR_SIZE as u32 || signature != [0x55, 0xAA] {
        return None;
    }

    let first_data_sector = reserved_sector_count + num_fats * fat_sz32;
    Some(Bpb { sectors_per_cluster, first_data_sector, fat_begin_sector: reserved_sector_count, root_cluster })
}

/// Follows one link in a cluster chain via the FAT (4 bytes/entry,
/// masked to 28 bits). `None` at end-of-chain (`>= FAT32_EOC_MIN`).
fn next_cluster(bpb: &Bpb, storage_slot: u32, storage_reply: u32, cluster: u32) -> Option<u32> {
    let fat_offset = cluster * 4;
    let fat_sector = bpb.fat_begin_sector + fat_offset / SECTOR_SIZE as u32;
    let ent_offset = (fat_offset % SECTOR_SIZE as u32) as usize;

    if !read_sector(storage_slot, storage_reply, fat_sector) {
        return None;
    }
    let b = storage_buf();
    let raw = u32::from_le_bytes([b[ent_offset], b[ent_offset + 1], b[ent_offset + 2], b[ent_offset + 3]]) & 0x0FFF_FFFF;
    if raw >= FAT32_EOC_MIN || raw < 2 {
        None
    } else {
        Some(raw)
    }
}

/// Walks the root directory's cluster chain looking for `name` (already
/// in FAT's fixed 11-byte 8.3 form). Returns `(start_cluster, size)` if
/// found; skips long-filename (`attr == 0x0F`) and volume-label
/// (`attr & 0x08 != 0`) entries, stops at the first `0x00` (end of
/// directory) entry.
fn find_in_root(bpb: &Bpb, storage_slot: u32, storage_reply: u32, name: &[u8; 11]) -> Option<(u32, u32)> {
    let mut cluster = bpb.root_cluster;
    loop {
        let base_sector = bpb.cluster_to_sector(cluster);
        for s in 0..bpb.sectors_per_cluster {
            if !read_sector(storage_slot, storage_reply, base_sector + s) {
                return None;
            }
            let b = storage_buf();
            for e in 0..(SECTOR_SIZE / DIRENT_SIZE) {
                let off = e * DIRENT_SIZE;
                let first = b[off];
                if first == 0x00 {
                    return None;
                }
                if first == 0xE5 {
                    continue;
                }
                let attr = b[off + 11];
                if attr == 0x0F || attr & 0x08 != 0 {
                    continue;
                }
                if &b[off..off + 11] == name {
                    let cluster_hi = u16::from_le_bytes([b[off + 20], b[off + 21]]) as u32;
                    let cluster_lo = u16::from_le_bytes([b[off + 26], b[off + 27]]) as u32;
                    let size = u32::from_le_bytes([b[off + 28], b[off + 29], b[off + 30], b[off + 31]]);
                    return Some(((cluster_hi << 16) | cluster_lo, size));
                }
            }
        }
        match next_cluster(bpb, storage_slot, storage_reply, cluster) {
            Some(c) => cluster = c,
            None => return None,
        }
    }
}

/// Reads up to one sector's worth of `file` data at `offset` into
/// `client_buf()`. Returns the number of bytes placed there (`0` = EOF).
fn read_file(bpb: &Bpb, storage_slot: u32, storage_reply: u32, file: &OpenFile, offset: u32, len: u32) -> u32 {
    if offset >= file.size {
        return 0;
    }
    let remaining_in_file = file.size - offset;
    let cluster_size = bpb.cluster_size_bytes();
    let cluster_index = offset / cluster_size;
    let offset_in_cluster = offset % cluster_size;
    let sector_in_cluster = offset_in_cluster / SECTOR_SIZE as u32;
    let offset_in_sector = (offset_in_cluster % SECTOR_SIZE as u32) as usize;

    let mut cluster = file.start_cluster;
    for _ in 0..cluster_index {
        match next_cluster(bpb, storage_slot, storage_reply, cluster) {
            Some(c) => cluster = c,
            None => return 0,
        }
    }

    let sector = bpb.cluster_to_sector(cluster) + sector_in_cluster;
    if !read_sector(storage_slot, storage_reply, sector) {
        return 0;
    }

    let avail_in_sector = (SECTOR_SIZE - offset_in_sector) as u32;
    let n = len.min(avail_in_sector).min(remaining_in_file) as usize;
    let src = storage_buf();
    client_buf()[..n].copy_from_slice(&src[offset_in_sector..offset_in_sector + n]);
    n as u32
}

#[no_mangle]
#[link_section = ".text.start"]
pub extern "C" fn _start() -> ! {
    let storage_slot = match libpcern::lookup_name_retry(b"storage", MY_INBOX, 1000) {
        Some(s) => s,
        None => libpcern::exit(1),
    };

    // A dedicated endpoint for storage's replies, separate from MY_INBOX
    // (which serves this task's own fs clients) -- see the doc comment
    // on MY_INBOX above for why.
    let storage_reply = libpcern::endpoint_create();

    let storage_buf_grant = libpcern::mem_alloc(STORAGE_BUF_VIRT);
    if storage_buf_grant == 0 {
        libpcern::exit(1);
    }
    libpcern::storage_connect(storage_slot, storage_buf_grant, storage_reply);

    let bpb = match parse_bpb(storage_slot, storage_reply) {
        Some(bpb) => bpb,
        // No usable disk behind storage_ata (or it's not FAT32) -- stay
        // alive and idle rather than exit, same as storage_ata itself
        // does when nobody's asked it for anything; just never register
        // "fs", so lookups for it honestly report "not found" forever.
        None => loop {
            libpcern::yield_now();
        },
    };

    libpcern::register_name(b"fs", MY_INBOX);

    let mut client_buf_mapped = false;
    let mut client_reply: u32 = 0;
    let mut pending_name_lo: [u8; 8] = [0; 8];
    let mut open_file: Option<OpenFile> = None;

    loop {
        let r = libpcern::recv(MY_INBOX);

        match r.w0 {
            libpcern::FS_OP_SET_BUFFER => {
                if r.transferred_slot != 0 && libpcern::map_memory(r.transferred_slot, CLIENT_BUF_VIRT) == 0 {
                    client_buf_mapped = true;
                }
            }
            libpcern::FS_OP_SET_REPLY => {
                if r.transferred_slot != 0 {
                    client_reply = r.transferred_slot;
                }
            }
            libpcern::FS_OP_OPEN_NAME1 => {
                pending_name_lo[0..4].copy_from_slice(&r.w1.to_le_bytes());
                pending_name_lo[4..8].copy_from_slice(&r.w2.to_le_bytes());
            }
            libpcern::FS_OP_OPEN_NAME2 => {
                if client_reply == 0 {
                    continue;
                }
                let tail = r.w1.to_le_bytes();
                let mut name = [0u8; 11];
                name[0..8].copy_from_slice(&pending_name_lo);
                name[8..11].copy_from_slice(&tail[0..3]);

                match find_in_root(&bpb, storage_slot, storage_reply, &name) {
                    Some((start_cluster, size)) => {
                        open_file = Some(OpenFile { start_cluster, size });
                        libpcern::send(client_reply, 1, size, 0, 0);
                    }
                    None => {
                        open_file = None;
                        libpcern::send(client_reply, 0, 0, 0, 0);
                    }
                }
            }
            libpcern::FS_OP_READ => {
                if client_reply == 0 {
                    continue;
                }
                if !client_buf_mapped {
                    libpcern::send(client_reply, 0, 0, 0, 0);
                    continue;
                }
                let n = match &open_file {
                    Some(file) => read_file(&bpb, storage_slot, storage_reply, file, r.w1, r.w2),
                    None => 0,
                };
                libpcern::send(client_reply, n, 0, 0, 0);
            }
            _ => {}
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
