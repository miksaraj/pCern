//! Checkpoint J: a read-only FAT32 filesystem server. Looks up "storage"
//! via the name service and reads sectors through it (Checkpoint I),
//! parses just enough of the BPB/FAT/root-directory structures to open a
//! root-directory 8.3-named file and read it back, then registers as
//! "fs" and serves the same kind of shared-memory-grant protocol to its
//! own clients that storage_ata serves to it (see libpcern's FS_OP_*/
//! fs_connect/fs_open/fs_read).
//!
//! Phase 7, Checkpoint Q adds write support: overwrite, growth (free-
//! cluster allocation + FAT chain extension), and brand-new file
//! creation. Every FAT32 entry update mirrors both FAT copies (see
//! `write_fat_entry`) -- unlike the read side, which deliberately only
//! ever consults the first copy (fine for reading something this task
//! itself trusts; leaving the second copy stale the moment this task
//! *writes* would be externally-visible drift the instant anything else
//! -- a real BIOS, `fsck.fat`, `mtools`, a dual-boot OS -- reads or
//! "fixes" this disk). Newly allocated *file-data* clusters are not
//! zero-filled: `read_file`'s bounds check already refuses to expose any
//! byte beyond the directory entry's recorded size, so leftover disk
//! garbage in an unwritten tail is never observable through `FS_OP_READ`
//! regardless of zeroing -- **as long as `size` only ever grows to cover a
//! contiguous range starting from what was already written**, which is
//! exactly what `write_file` enforces by refusing (`offset > file.size`)
//! any write that would leave a gap of never-written clusters standing in
//! for real content. Shrinking is deliberately not inferred from a write's
//! own coverage (a write in the middle of a file must never truncate
//! whatever comes after it) -- `FS_OP_TRUNCATE`/`truncate_file` is the only
//! way a file's size decreases, and it in turn refuses to *grow* past the
//! current size, for the same never-expose-unwritten-bytes reason.
//! Newly allocated *root-directory* clusters are the one exception and ARE
//! zero-filled (see `grow_root_for_free_slot`): unlike file data,
//! directory-walking scans raw bytes looking for a `0x00`/`0xE5` marker
//! with no separately-tracked size to bound it, so unzeroed garbage there
//! could be misread as a real (or falsely terminating) directory entry.
//!
//! Scope for v1 (deliberately narrow, matching this phase's other
//! scope-narrowing calls): root-directory files only, no subdirectory
//! traversal, 8.3 names only (long-filename and volume-label entries are
//! skipped, never matched), one client and one open file at a time. The
//! one FAT32-specific landmine worth calling out: the root directory is
//! itself an ordinary cluster chain rooted at `root_cluster` (not a fixed
//! region the way FAT16's root dir is), so it's walked with exactly the
//! same `next_cluster` logic as file data.
//!
//! Checkpoint U adds `find_fat32_base`: the FAT32 volume this task reads
//! and writes may now start at LBA 0 directly (the original
//! "superfloppy" layout, still what `make test-fat32-image` builds) *or*
//! wherever an MBR partition table's first FAT32 partition begins (the
//! installed boot disk `make disk` builds, which needs a real partition
//! table so GRUB's own `i386-pc` install has a gap to embed its
//! `core.img` in). Every other function in this file is unaware of the
//! difference -- `parse_bpb` bakes the base LBA into `fat_begin_sector`/
//! `first_data_sector` once, up front.

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
    /// Sectors per FAT copy -- the second copy begins at
    /// `fat_begin_sector + fat_sz32` (Checkpoint Q; the read side never
    /// needed this since it only ever consults the first copy).
    fat_sz32: u32,
    root_cluster: u32,
    /// Highest valid cluster number (`total_data_clusters + 1`, since
    /// cluster numbering starts at 2) -- bounds `alloc_cluster`'s
    /// free-cluster scan.
    max_cluster: u32,
}

impl Bpb {
    fn cluster_to_sector(&self, cluster: u32) -> u32 {
        self.first_data_sector + (cluster - 2) * self.sectors_per_cluster
    }

    fn cluster_size_bytes(&self) -> u32 {
        self.sectors_per_cluster * SECTOR_SIZE as u32
    }
}

/// Where a file's (or, for a freshly created file, a brand-new) directory
/// entry lives on disk, so a write that changes size/first-cluster can
/// persist that change back (Checkpoint Q).
#[derive(Clone, Copy)]
struct DirLoc {
    sector: u32,
    offset: usize,
}

struct OpenFile {
    start_cluster: u32,
    size: u32,
    dirent: DirLoc,
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

/// Writes `storage_buf()`'s current contents to `lba` (Checkpoint Q).
fn write_sector(storage_slot: u32, storage_reply: u32, lba: u32) -> bool {
    libpcern::storage_write_block(storage_slot, storage_reply, lba)
}

/// Checks whether `b` (a sector just placed in `storage_buf()` by
/// `read_sector`) looks like a genuine FAT32 -- not FAT12/FAT16 -- boot
/// sector: the fixed 512-byte-per-sector field, the 0x55AA boot
/// signature, and `BPB_FATSz32` (bytes 36-39) being nonzero. FAT12/16
/// instead records its FAT size in the 16-bit `BPB_FATSz16` field (bytes
/// 22-23) and leaves this 32-bit one zero -- the FAT spec's own
/// documented way to tell FAT32 apart from its predecessors.
/// Deliberately doesn't consult `BS_FilSysType` (bytes 82-90,
/// conventionally `"FAT32   "`): the spec calls that field advisory and
/// says implementations must not rely on it to determine filesystem
/// type, so a disk formatted by a tool that leaves it non-standard would
/// otherwise be rejected even though it's perfectly valid FAT32.
fn fat32_fields_valid(b: &[u8; SECTOR_SIZE]) -> bool {
    let bytes_per_sector = u16::from_le_bytes([b[11], b[12]]) as u32;
    let fat_sz32 = u32::from_le_bytes([b[36], b[37], b[38], b[39]]);
    bytes_per_sector == SECTOR_SIZE as u32 && [b[510], b[511]] == [0x55, 0xAA] && fat_sz32 != 0
}

/// Finds the LBA where the FAT32 volume's own boot sector actually
/// starts: `0` directly for an unpartitioned/"superfloppy" FAT32 disk, or
/// wherever an MBR partition table's first FAT32 partition begins.
/// Checkpoint U's installed boot disk needs a real MBR (GRUB's own
/// `i386-pc` BIOS install embeds its `core.img` in the gap between the
/// MBR and the first partition -- a bare FAT32 filesystem has no such
/// gap), while the pre-existing FAT32 test image and any older
/// unpartitioned disk still boot sector 0 directly -- both are supported
/// here rather than picking one, so neither the test harness nor any
/// disk built before this checkpoint needs to change.
///
/// Only the first partition table entry is consulted, and only if its
/// type byte is `0x0B` (FAT32 CHS) or `0x0C` (FAT32 LBA) -- this driver
/// has no use for, and doesn't need to understand, any other partition
/// type or a second partition. LBA 0's own bytes are only ever trusted as
/// a partition table after confirming its own 0x55AA boot signature is
/// present -- without that check, an uninitialized/garbage disk could
/// have its incidental bytes at the partition-type offset coincidentally
/// match `0x0B`/`0x0C` and send a bogus LBA into a real disk read.
/// Returns `None` if neither a direct FAT32 boot sector nor a partition
/// table entry pointing at one is found. On success, `storage_buf()`
/// holds the returned LBA's own bytes (the last sector this function
/// read), so callers don't need to re-read it.
fn find_fat32_base(storage_slot: u32, storage_reply: u32) -> Option<u32> {
    if !read_sector(storage_slot, storage_reply, 0) {
        return None;
    }
    if fat32_fields_valid(storage_buf()) {
        return Some(0);
    }

    let b = storage_buf();
    if [b[510], b[511]] != [0x55, 0xAA] {
        return None;
    }
    const PART1_OFFSET: usize = 446;
    let part_type = b[PART1_OFFSET + 4];
    if part_type != 0x0B && part_type != 0x0C {
        return None;
    }
    let start_lba = u32::from_le_bytes([
        b[PART1_OFFSET + 8],
        b[PART1_OFFSET + 9],
        b[PART1_OFFSET + 10],
        b[PART1_OFFSET + 11],
    ]);

    if !read_sector(storage_slot, storage_reply, start_lba) {
        return None;
    }
    if fat32_fields_valid(storage_buf()) {
        Some(start_lba)
    } else {
        None
    }
}

/// Returns `None` if no FAT32 volume can be found at all (no disk
/// attached behind storage_ata -- this task stays alive but never
/// registers "fs" rather than treating that as fatal, the same
/// graceful-idle behavior storage_ata itself already has when nobody's
/// asked it to do anything) or doesn't look like a valid FAT32 BPB.
fn parse_bpb(storage_slot: u32, storage_reply: u32) -> Option<Bpb> {
    // `find_fat32_base` already read the winning candidate sector as the
    // last thing it did before returning, so `storage_buf()` already
    // holds its bytes -- no need to read it again.
    let base = find_fat32_base(storage_slot, storage_reply)?;
    let b = storage_buf();
    let bytes_per_sector = u16::from_le_bytes([b[11], b[12]]) as u32;
    let sectors_per_cluster = b[13] as u32;
    let reserved_sector_count = u16::from_le_bytes([b[14], b[15]]) as u32;
    let num_fats = b[16] as u32;
    let total_sectors32 = u32::from_le_bytes([b[32], b[33], b[34], b[35]]);
    let fat_sz32 = u32::from_le_bytes([b[36], b[37], b[38], b[39]]);
    let root_cluster = u32::from_le_bytes([b[44], b[45], b[46], b[47]]);
    let signature = [b[510], b[511]];

    if bytes_per_sector != SECTOR_SIZE as u32 || signature != [0x55, 0xAA] {
        return None;
    }

    // `total_sectors32`/the reserved+FAT area size are volume-relative
    // (as FAT32 always records them); `base` only needs adding to the two
    // fields below, since those are the ones later code uses as absolute
    // LBAs passed straight to `read_sector`/`write_sector`.
    let first_data_sector = reserved_sector_count + num_fats * fat_sz32;
    // A corrupt or crafted BPB (reachable, since Checkpoint U's MBR
    // lookup above can point this at any on-disk sector, not just a
    // trusted fixed LBA 0) could otherwise underflow this subtraction or
    // divide by zero -- both refused here rather than panicking or
    // wrapping into a bogus `max_cluster` that would corrupt every later
    // bounds check in `alloc_cluster`'s free-cluster scan.
    if sectors_per_cluster == 0 || total_sectors32 < first_data_sector {
        return None;
    }
    let max_cluster = (total_sectors32 - first_data_sector) / sectors_per_cluster + 1;
    Some(Bpb {
        sectors_per_cluster,
        first_data_sector: base + first_data_sector,
        fat_begin_sector: base + reserved_sector_count,
        fat_sz32,
        root_cluster,
        max_cluster,
    })
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

/// Reads the raw (masked, 28-bit) FAT entry for `cluster` without the
/// end-of-chain/free interpretation `next_cluster` applies -- used by
/// `alloc_cluster`'s free-cluster scan, where `0` (free) and a genuine
/// chain link both need to be told apart from "end of chain".
fn read_fat_entry(bpb: &Bpb, storage_slot: u32, storage_reply: u32, cluster: u32) -> Option<u32> {
    let fat_offset = cluster * 4;
    let fat_sector = bpb.fat_begin_sector + fat_offset / SECTOR_SIZE as u32;
    let ent_offset = (fat_offset % SECTOR_SIZE as u32) as usize;
    if !read_sector(storage_slot, storage_reply, fat_sector) {
        return None;
    }
    let b = storage_buf();
    Some(u32::from_le_bytes([b[ent_offset], b[ent_offset + 1], b[ent_offset + 2], b[ent_offset + 3]]) & 0x0FFF_FFFF)
}

/// Writes `cluster`'s FAT entry to `value` (28 bits; the top 4 reserved
/// bits of whatever was already on disk are preserved, not clobbered) in
/// *both* FAT copies -- see the module doc comment for why writes, unlike
/// reads, don't narrow to just the first copy.
fn write_fat_entry(bpb: &Bpb, storage_slot: u32, storage_reply: u32, cluster: u32, value: u32) -> bool {
    let fat_offset = cluster * 4;
    let sector_in_fat = fat_offset / SECTOR_SIZE as u32;
    let ent_offset = (fat_offset % SECTOR_SIZE as u32) as usize;

    for fat_base in [bpb.fat_begin_sector, bpb.fat_begin_sector + bpb.fat_sz32] {
        let sector = fat_base + sector_in_fat;
        if !read_sector(storage_slot, storage_reply, sector) {
            return false;
        }
        let b = storage_buf();
        let old = u32::from_le_bytes([b[ent_offset], b[ent_offset + 1], b[ent_offset + 2], b[ent_offset + 3]]);
        let new = (old & 0xF000_0000) | (value & 0x0FFF_FFFF);
        b[ent_offset..ent_offset + 4].copy_from_slice(&new.to_le_bytes());
        if !write_sector(storage_slot, storage_reply, sector) {
            return false;
        }
    }
    true
}

/// Linear free-cluster scan (first FAT entry `== 0`), starting at cluster
/// 2. Immediately marks the found cluster end-of-chain in both FAT copies
/// to claim it before returning, so a second allocation can't also pick
/// it. `None` if the disk is full.
fn alloc_cluster(bpb: &Bpb, storage_slot: u32, storage_reply: u32) -> Option<u32> {
    for cluster in 2..=bpb.max_cluster {
        match read_fat_entry(bpb, storage_slot, storage_reply, cluster) {
            Some(0) => {
                if write_fat_entry(bpb, storage_slot, storage_reply, cluster, FAT32_EOC_MIN) {
                    return Some(cluster);
                }
                return None;
            }
            Some(_) => continue,
            None => return None,
        }
    }
    None
}

/// Walks the root directory's cluster chain looking for `name` (already
/// in FAT's fixed 11-byte 8.3 form). Returns `(start_cluster, size,
/// dirent location)` if found; skips long-filename (`attr == 0x0F`) and
/// volume-label (`attr & 0x08 != 0`) entries, stops at the first `0x00`
/// (end of directory) entry.
fn find_in_root(bpb: &Bpb, storage_slot: u32, storage_reply: u32, name: &[u8; 11]) -> Option<(u32, u32, DirLoc)> {
    let mut cluster = bpb.root_cluster;
    loop {
        let base_sector = bpb.cluster_to_sector(cluster);
        for s in 0..bpb.sectors_per_cluster {
            let sector = base_sector + s;
            if !read_sector(storage_slot, storage_reply, sector) {
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
                    return Some(((cluster_hi << 16) | cluster_lo, size, DirLoc { sector, offset: off }));
                }
            }
        }
        match next_cluster(bpb, storage_slot, storage_reply, cluster) {
            Some(c) => cluster = c,
            None => return None,
        }
    }
}

/// Finds the first free directory-entry slot (`0x00` end-of-directory
/// marker, or a deleted `0xE5` entry) in the root directory's cluster
/// chain, extending the chain by one cluster if every existing cluster is
/// full. Unlike file-data clusters, a newly allocated root-directory
/// cluster IS zero-filled (see the module doc comment for why) before its
/// first entry is returned as the free slot.
fn find_free_root_slot(bpb: &Bpb, storage_slot: u32, storage_reply: u32) -> Option<DirLoc> {
    let mut cluster = bpb.root_cluster;
    loop {
        let base_sector = bpb.cluster_to_sector(cluster);
        for s in 0..bpb.sectors_per_cluster {
            let sector = base_sector + s;
            if !read_sector(storage_slot, storage_reply, sector) {
                return None;
            }
            let b = storage_buf();
            for e in 0..(SECTOR_SIZE / DIRENT_SIZE) {
                let off = e * DIRENT_SIZE;
                if b[off] == 0x00 || b[off] == 0xE5 {
                    return Some(DirLoc { sector, offset: off });
                }
            }
        }
        match next_cluster(bpb, storage_slot, storage_reply, cluster) {
            Some(c) => cluster = c,
            None => {
                let new_cluster = alloc_cluster(bpb, storage_slot, storage_reply)?;
                if !write_fat_entry(bpb, storage_slot, storage_reply, cluster, new_cluster) {
                    return None;
                }
                let zero = [0u8; SECTOR_SIZE];
                let new_base = bpb.cluster_to_sector(new_cluster);
                for s in 0..bpb.sectors_per_cluster {
                    storage_buf().copy_from_slice(&zero);
                    if !write_sector(storage_slot, storage_reply, new_base + s) {
                        return None;
                    }
                }
                return Some(DirLoc { sector: new_base, offset: 0 });
            }
        }
    }
}

/// Persists a directory entry's size and first-cluster fields
/// (read-modify-write, since a dirent is 32 of a sector's 512 bytes).
fn write_dirent_update(storage_slot: u32, storage_reply: u32, loc: DirLoc, start_cluster: u32, size: u32) -> bool {
    if !read_sector(storage_slot, storage_reply, loc.sector) {
        return false;
    }
    let b = storage_buf();
    let off = loc.offset;
    b[off + 20..off + 22].copy_from_slice(&((start_cluster >> 16) as u16).to_le_bytes());
    b[off + 26..off + 28].copy_from_slice(&((start_cluster & 0xFFFF) as u16).to_le_bytes());
    b[off + 28..off + 32].copy_from_slice(&size.to_le_bytes());
    write_sector(storage_slot, storage_reply, loc.sector)
}

/// Writes a brand-new 8.3 directory entry (regular file, zero length, no
/// cluster allocated yet -- the standard FAT32 empty-file convention) at
/// `loc`. Timestamps are left zeroed: nothing in this project reads them,
/// and mtools/a real FAT32 driver tolerate a zeroed creation
/// date/time rather than refusing to read the entry.
fn write_new_dirent(storage_slot: u32, storage_reply: u32, loc: DirLoc, name: &[u8; 11]) -> bool {
    if !read_sector(storage_slot, storage_reply, loc.sector) {
        return false;
    }
    let b = storage_buf();
    let off = loc.offset;
    b[off..off + DIRENT_SIZE].fill(0);
    b[off..off + 11].copy_from_slice(name);
    b[off + 11] = 0x20; // ATTR_ARCHIVE
    write_sector(storage_slot, storage_reply, loc.sector)
}

/// Opens `name`, creating a fresh zero-length file (a new root-directory
/// entry, no cluster allocated) if `create` is set and it doesn't already
/// exist. Returns `(start_cluster, size, dirent location)`.
fn open_or_create(
    bpb: &Bpb,
    storage_slot: u32,
    storage_reply: u32,
    name: &[u8; 11],
    create: bool,
) -> Option<(u32, u32, DirLoc)> {
    if let Some(found) = find_in_root(bpb, storage_slot, storage_reply, name) {
        return Some(found);
    }
    if !create {
        return None;
    }
    let loc = find_free_root_slot(bpb, storage_slot, storage_reply)?;
    if !write_new_dirent(storage_slot, storage_reply, loc, name) {
        return None;
    }
    Some((0, 0, loc))
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

/// Writes up to one sector's worth of `client_buf()` bytes at `offset`
/// into `file`, allocating/extending its cluster chain as needed and
/// persisting the resulting size/first-cluster to its directory entry.
/// Returns the number of bytes actually written (`0` = `offset` is beyond
/// the file's current size, the disk ran out of free clusters, or a
/// sector read/write failed).
fn write_file(bpb: &Bpb, storage_slot: u32, storage_reply: u32, file: &mut OpenFile, offset: u32, len: u32) -> u32 {
    // `offset` may never exceed the current size: a write starting past
    // the end would leave a gap of newly-allocated-but-never-written
    // clusters standing in for it, and those clusters are deliberately not
    // zero-filled (see the module doc comment) -- exposing whatever old
    // data already occupies them the moment `file.size` grows to cover
    // that gap. Refusing any offset beyond the current end keeps every
    // byte between 0 and the new size something this call (or an earlier
    // one) actually wrote. A legitimate caller either overwrites within
    // the existing range (offset < file.size) or appends immediately at
    // the end (offset == file.size); it never needs to skip ahead.
    if offset > file.size {
        return 0;
    }
    let cluster_size = bpb.cluster_size_bytes();
    let cluster_index = offset / cluster_size;
    let offset_in_cluster = offset % cluster_size;
    let sector_in_cluster = offset_in_cluster / SECTOR_SIZE as u32;
    let offset_in_sector = (offset_in_cluster % SECTOR_SIZE as u32) as usize;

    let mut cluster = if file.start_cluster == 0 {
        let c = match alloc_cluster(bpb, storage_slot, storage_reply) {
            Some(c) => c,
            None => return 0,
        };
        file.start_cluster = c;
        c
    } else {
        file.start_cluster
    };

    for _ in 0..cluster_index {
        cluster = match next_cluster(bpb, storage_slot, storage_reply, cluster) {
            Some(c) => c,
            None => {
                let new_cluster = match alloc_cluster(bpb, storage_slot, storage_reply) {
                    Some(c) => c,
                    None => return 0,
                };
                if !write_fat_entry(bpb, storage_slot, storage_reply, cluster, new_cluster) {
                    return 0;
                }
                new_cluster
            }
        };
    }

    let sector = bpb.cluster_to_sector(cluster) + sector_in_cluster;
    // Read-modify-write: preserves whatever else is in this sector,
    // whether pre-existing file content (an overwrite) or -- for a
    // freshly allocated cluster -- unzeroed disk garbage outside this
    // write's range, which is fine since read_file's bounds check on
    // `size` never exposes it (see the module doc comment).
    if !read_sector(storage_slot, storage_reply, sector) {
        return 0;
    }
    let avail_in_sector = (SECTOR_SIZE - offset_in_sector) as u32;
    let n = len.min(avail_in_sector) as usize;
    let dst = storage_buf();
    dst[offset_in_sector..offset_in_sector + n].copy_from_slice(&client_buf()[..n]);
    if !write_sector(storage_slot, storage_reply, sector) {
        return 0;
    }

    let new_end = offset + n as u32;
    if new_end > file.size {
        file.size = new_end;
    }
    if !write_dirent_update(storage_slot, storage_reply, file.dirent, file.start_cluster, file.size) {
        return 0;
    }
    n as u32
}

/// Shrinks `file`'s recorded size to `new_size`, persisting the change to
/// its directory entry. The only way a file's size ever decreases --
/// `write_file` is grow-or-overwrite-only by design (see its own doc
/// comment). Refuses (returns `0`) to grow past the current size: nothing
/// here can guarantee bytes between the old and new size were actually
/// written, and exposing them anyway would reopen the same gap
/// `write_file`'s own offset bound exists to prevent. Returns `1` on
/// success.
fn truncate_file(storage_slot: u32, storage_reply: u32, file: &mut OpenFile, new_size: u32) -> u32 {
    if new_size > file.size {
        return 0;
    }
    file.size = new_size;
    if !write_dirent_update(storage_slot, storage_reply, file.dirent, file.start_cluster, file.size) {
        return 0;
    }
    1
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

                let create = r.w2 != 0;
                match open_or_create(&bpb, storage_slot, storage_reply, &name, create) {
                    Some((start_cluster, size, dirent)) => {
                        open_file = Some(OpenFile { start_cluster, size, dirent });
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
            libpcern::FS_OP_WRITE => {
                if client_reply == 0 {
                    continue;
                }
                if !client_buf_mapped {
                    libpcern::send(client_reply, 0, 0, 0, 0);
                    continue;
                }
                let n = match &mut open_file {
                    Some(file) => write_file(&bpb, storage_slot, storage_reply, file, r.w1, r.w2),
                    None => 0,
                };
                libpcern::send(client_reply, n, 0, 0, 0);
            }
            libpcern::FS_OP_TRUNCATE => {
                if client_reply == 0 {
                    continue;
                }
                let ok = match &mut open_file {
                    Some(file) => truncate_file(storage_slot, storage_reply, file, r.w1),
                    None => 0,
                };
                libpcern::send(client_reply, ok, 0, 0, 0);
            }
            _ => {}
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    libpcern::exit(1);
}
