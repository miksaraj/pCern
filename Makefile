CARGO := cargo
PROFILE := release
TARGET := i686-pcern
KERNEL_DIR := kernel
KERNEL_BIN := $(KERNEL_DIR)/target/$(TARGET)/$(PROFILE)/pcern

OBJCOPY := objcopy
USERLAND_DIR := userland

CONSOLE_SERVER_DIR := $(USERLAND_DIR)/drivers/console_server
CONSOLE_SERVER_TARGET := i686-pcern-user
CONSOLE_SERVER_ELF := $(CONSOLE_SERVER_DIR)/target/$(CONSOLE_SERVER_TARGET)/$(PROFILE)/console_server
CONSOLE_SERVER_BIN := $(USERLAND_DIR)/console_server.bin

NAMESERVICE_DIR := $(USERLAND_DIR)/services/nameservice
NAMESERVICE_TARGET := i686-pcern-user
NAMESERVICE_ELF := $(NAMESERVICE_DIR)/target/$(NAMESERVICE_TARGET)/$(PROFILE)/nameservice
NAMESERVICE_BIN := $(USERLAND_DIR)/nameservice.bin

# Regression test fixtures (see userland/cap_test) covering capability
# transfer/revocation, shared memory grants, and the storage/fs
# protocols -- not part of the default `iso`/`userland` build. Built via
# `make cap_test` on their own, or use `make test` to build, boot, and
# check all of them automatically (see run_tests.sh).
CAP_TEST_DIR := $(USERLAND_DIR)/cap_test
CAP_TEST_TARGET := i686-pcern-user

STORAGE_ATA_DIR := $(USERLAND_DIR)/drivers/storage_ata
STORAGE_ATA_TARGET := i686-pcern-user
STORAGE_ATA_ELF := $(STORAGE_ATA_DIR)/target/$(STORAGE_ATA_TARGET)/$(PROFILE)/storage_ata
STORAGE_ATA_BIN := $(USERLAND_DIR)/storage_ata.bin

FS_FAT32_DIR := $(USERLAND_DIR)/services/fs_fat32
FS_FAT32_TARGET := i686-pcern-user
FS_FAT32_ELF := $(FS_FAT32_DIR)/target/$(FS_FAT32_TARGET)/$(PROFILE)/fs_fat32
FS_FAT32_BIN := $(USERLAND_DIR)/fs_fat32.bin

NET_RTL8139_DIR := $(USERLAND_DIR)/drivers/net_rtl8139
NET_RTL8139_TARGET := i686-pcern-user
NET_RTL8139_ELF := $(NET_RTL8139_DIR)/target/$(NET_RTL8139_TARGET)/$(PROFILE)/net_rtl8139
NET_RTL8139_BIN := $(USERLAND_DIR)/net_rtl8139.bin

NETSTACK_DIR := $(USERLAND_DIR)/services/netstack
NETSTACK_TARGET := i686-pcern-user
NETSTACK_ELF := $(NETSTACK_DIR)/target/$(NETSTACK_TARGET)/$(PROFILE)/netstack
NETSTACK_BIN := $(USERLAND_DIR)/netstack.bin

SHELL_DIR := $(USERLAND_DIR)/bin/shell
SHELL_TARGET := i686-pcern-user
SHELL_ELF := $(SHELL_DIR)/target/$(SHELL_TARGET)/$(PROFILE)/shell
SHELL_BIN := $(USERLAND_DIR)/shell.bin

# Every default userland service, in main.rs's fixed spawn order. `iso`
# iterates this list (instead of hand-listing every binary separately) so
# a future service only needs adding here once, not as two separately
# hand-kept copies (this list, and `disk`'s below) that could silently
# drift apart.
PROD_USERLAND_BINS := $(NAMESERVICE_BIN) $(CONSOLE_SERVER_BIN) $(STORAGE_ATA_BIN) $(FS_FAT32_BIN) $(SHELL_BIN) $(NET_RTL8139_BIN) $(NETSTACK_BIN)
# `disk`'s own copy of the same list, paired (`path:8.3-name`) with the
# uppercase short name each binary lands under on the FAT32 boot disk --
# unlike `iso`'s ISO9660+Rock Ridge (which tolerates the binaries'
# ordinary lowercase basenames above), the FAT32 partition GRUB's `fat`
# module and this project's own `fs_fat32` both read only supports
# classic 8.3 short names. Kept as one paired list, not two separate
# parallel ones, so there's no way for the two to fall out of sync with
# each other by reordering.
PROD_USERLAND_DISK_FILES := $(NAMESERVICE_BIN):NAMESERV.BIN $(CONSOLE_SERVER_BIN):CONSOLE.BIN $(STORAGE_ATA_BIN):STORAGE.BIN $(FS_FAT32_BIN):FS_FAT32.BIN $(SHELL_BIN):SHELL.BIN $(NET_RTL8139_BIN):RTL8139.BIN $(NETSTACK_BIN):NETSTACK.BIN

CP := cp
RM := rm -rf
MKDIR := mkdir -pv

CFG := $(KERNEL_DIR)/grub.cfg
ISO_PATH := iso
BOOT_PATH := $(ISO_PATH)/boot
GRUB_PATH := $(BOOT_PATH)/grub
ISO := zephyrlite-i386.iso

# Test harness (see `make test`): a separate kernel build (--features
# test_harness) + grub config + ISO, so the production `iso`/`kernel`
# targets above are completely untouched by any of this.
CFG_TEST := $(KERNEL_DIR)/grub-test.cfg
ISO_TEST_PATH := iso-test
BOOT_TEST_PATH := $(ISO_TEST_PATH)/boot
GRUB_TEST_PATH := $(BOOT_TEST_PATH)/grub
ISO_TEST := pcern-test-i386.iso

# Checkpoint L's keyboard-input test harness: its own standalone kernel
# build (--features keyboard_test) + grub config + ISO, separate from
# both the production and the shared iso-test builds above -- see
# console_input_test.rs's doc comment for why this fixture can't share
# either of those.
CFG_KEYTEST := $(KERNEL_DIR)/grub-keytest.cfg
ISO_KEYTEST_PATH := iso-keytest
BOOT_KEYTEST_PATH := $(ISO_KEYTEST_PATH)/boot
GRUB_KEYTEST_PATH := $(BOOT_KEYTEST_PATH)/grub
ISO_KEYTEST := pcern-keytest-i386.iso

# Phase 7, Checkpoint R's raw-input test harness: its own standalone
# kernel build (--features raw_input_test) + grub config + ISO, same
# reason as keyboard_test's above -- see raw_input_test.rs's doc comment.
CFG_RAWTEST := $(KERNEL_DIR)/grub-rawtest.cfg
ISO_RAWTEST_PATH := iso-rawtest
BOOT_RAWTEST_PATH := $(ISO_RAWTEST_PATH)/boot
GRUB_RAWTEST_PATH := $(BOOT_RAWTEST_PATH)/grub
ISO_RAWTEST := pcern-rawtest-i386.iso

# Phase 7, Checkpoint S's full-screen editor test harness: its own
# standalone kernel build (--features editor_test) + grub config + ISO,
# same reason as the other *_test harnesses above -- see
# editor_input_test.rs's doc comment. Unlike keyboard_test/raw_input_test,
# this one needs the shared FAT32 test image attached (it exercises real
# fs_fat32 write support), same as the main iso-test build.
CFG_EDITORTEST := $(KERNEL_DIR)/grub-editortest.cfg
ISO_EDITORTEST_PATH := iso-editortest
BOOT_EDITORTEST_PATH := $(ISO_EDITORTEST_PATH)/boot
GRUB_EDITORTEST_PATH := $(BOOT_EDITORTEST_PATH)/grub
ISO_EDITORTEST := pcern-editortest-i386.iso

# Checkpoint V's reboot-syscall test harness: its own standalone kernel
# build (--features reboot_test) + grub config + ISO, same reason as the
# other *_test harnesses above -- see run_reboot_test.sh's doc comment.
CFG_REBOOTTEST := $(KERNEL_DIR)/grub-reboottest.cfg
ISO_REBOOTTEST_PATH := iso-reboottest
BOOT_REBOOTTEST_PATH := $(ISO_REBOOTTEST_PATH)/boot
GRUB_REBOOTTEST_PATH := $(BOOT_REBOOTTEST_PATH)/grub
ISO_REBOOTTEST := pcern-reboottest-i386.iso

# Checkpoint W's RTL8139 NIC-driver test harness: its own standalone
# kernel build (--features nic_test) + grub config + ISO, same reason as
# the other *_test harnesses above -- see run_nic_test.sh's doc comment.
# Unlike them, this one still needs net_rtl8139 itself present (built by
# `userland` like every other default service), since that's the thing
# actually being tested.
CFG_NICTEST := $(KERNEL_DIR)/grub-nictest.cfg
ISO_NICTEST_PATH := iso-nictest
BOOT_NICTEST_PATH := $(ISO_NICTEST_PATH)/boot
GRUB_NICTEST_PATH := $(BOOT_NICTEST_PATH)/grub
ISO_NICTEST := pcern-nictest-i386.iso

# Checkpoint X's ARP/IPv4/ICMP responder test harness: its own standalone
# kernel build (--features arp_icmp_test) + grub config + ISO, same
# reason as the other *_test harnesses above -- see
# run_arp_icmp_test.sh's doc comment. Needs both net_rtl8139 and
# netstack present (built by `userland` like every other default
# service), since netstack -- the thing actually being tested -- is only
# reachable through it.
CFG_ARPTEST := $(KERNEL_DIR)/grub-arptest.cfg
ISO_ARPTEST_PATH := iso-arptest
BOOT_ARPTEST_PATH := $(ISO_ARPTEST_PATH)/boot
GRUB_ARPTEST_PATH := $(BOOT_ARPTEST_PATH)/grub
ISO_ARPTEST := pcern-arptest-i386.iso

# Checkpoint Y's TCP-client test harness: its own standalone kernel build
# (--features tcp_test) + grub config + ISO, same reason as the other
# *_test harnesses above -- see run_tcp_test.sh's doc comment. Needs
# net_rtl8139, netstack, and (unlike arp_icmp_test) an actual in-guest
# fixture again (http_client_test, built by `cap_test` since it's a
# regression fixture, not a default service).
CFG_TCPTEST := $(KERNEL_DIR)/grub-tcptest.cfg
ISO_TCPTEST_PATH := iso-tcptest
BOOT_TCPTEST_PATH := $(ISO_TCPTEST_PATH)/boot
GRUB_TCPTEST_PATH := $(BOOT_TCPTEST_PATH)/grub
ISO_TCPTEST := pcern-tcptest-i386.iso

# Checkpoint K: a host-built FAT32 image for end-to-end fs_fat32 testing
# (see userland/cap_test/src/bin/fs_client_test.rs), generated on demand
# via `make test-fat32-image` from the small tracked source files in
# testdata/ rather than committing the image itself -- attach it the same
# way Checkpoint I's spike disk was attached:
# `qemu-system-i386 ... -boot d -drive file=$(TEST_FAT32_IMG),if=ide,index=0,format=raw`.
MTOOLS_MFORMAT := mformat
MTOOLS_MCOPY := mcopy
TESTDATA_DIR := testdata
TEST_FAT32_IMG := test_fat32.img

# Checkpoint U: an installed FAT32 boot disk -- `make disk`'s whole point
# is that this is a real, writable disk image GRUB itself boots from (see
# `run-disk`/`test-disk-boot` below), not the read-only `iso`/ISO9660
# tooling above. Unlike the "superfloppy" FAT32 test image above (a bare
# FAT32 filesystem at LBA 0, no partition table), this disk needs a real
# MBR partition table: GRUB's `i386-pc` BIOS install embeds its own
# core.img in the gap between the MBR and the first partition, and a bare
# FAT32 filesystem has no such gap for it to use (see fs_fat32's own
# CHANGELOG/find_fat32_base for how it stays compatible with both
# layouts). GRUB_BIOS_SETUP's path is Debian/Ubuntu-specific
# (grub-pc-bin's layout) -- override it on other distros the same way the
# README already flags grub-mkrescue's own Debian/Ubuntu-centric default.
DISK_IMG := zephyrlite-i386.img
DISK_SIZE_MB := 64
DISK_PART_START_SECTOR := 2048
DISK_BUILD_DIR := disk-build
DISK_FAT_IMG := disk-fat.img
CFG_DISK := $(KERNEL_DIR)/grub-disk.cfg
GRUB_I386_PC_MODULES := fat multiboot part_msdos biosdisk normal serial terminal
GRUB_BIOS_SETUP := /usr/lib/grub/i386-pc/grub-bios-setup

.PHONY: all
all: iso
	@echo Make has completed.

.PHONY: kernel
kernel:
	cd $(KERNEL_DIR) && $(CARGO) build --$(PROFILE)
	grub-file --is-x86-multiboot $(KERNEL_BIN)

.PHONY: userland
userland: $(CONSOLE_SERVER_BIN) $(NAMESERVICE_BIN) $(STORAGE_ATA_BIN) $(FS_FAT32_BIN) $(NET_RTL8139_BIN) $(NETSTACK_BIN) $(SHELL_BIN)

$(CONSOLE_SERVER_BIN): FORCE
	cd $(CONSOLE_SERVER_DIR) && $(CARGO) build --$(PROFILE)
	$(OBJCOPY) -O binary --set-section-flags .bss=alloc,load,contents $(CONSOLE_SERVER_ELF) $(CONSOLE_SERVER_BIN)

$(NAMESERVICE_BIN): FORCE
	cd $(NAMESERVICE_DIR) && $(CARGO) build --$(PROFILE)
	$(OBJCOPY) -O binary --set-section-flags .bss=alloc,load,contents $(NAMESERVICE_ELF) $(NAMESERVICE_BIN)

$(STORAGE_ATA_BIN): FORCE
	cd $(STORAGE_ATA_DIR) && $(CARGO) build --$(PROFILE)
	$(OBJCOPY) -O binary --set-section-flags .bss=alloc,load,contents $(STORAGE_ATA_ELF) $(STORAGE_ATA_BIN)

$(FS_FAT32_BIN): FORCE
	cd $(FS_FAT32_DIR) && $(CARGO) build --$(PROFILE)
	$(OBJCOPY) -O binary --set-section-flags .bss=alloc,load,contents $(FS_FAT32_ELF) $(FS_FAT32_BIN)

$(NET_RTL8139_BIN): FORCE
	cd $(NET_RTL8139_DIR) && $(CARGO) build --$(PROFILE)
	$(OBJCOPY) -O binary --set-section-flags .bss=alloc,load,contents $(NET_RTL8139_ELF) $(NET_RTL8139_BIN)

$(NETSTACK_BIN): FORCE
	cd $(NETSTACK_DIR) && $(CARGO) build --$(PROFILE)
	$(OBJCOPY) -O binary --set-section-flags .bss=alloc,load,contents $(NETSTACK_ELF) $(NETSTACK_BIN)

$(SHELL_BIN): FORCE
	cd $(SHELL_DIR) && $(CARGO) build --$(PROFILE)
	$(OBJCOPY) -O binary --set-section-flags .bss=alloc,load,contents $(SHELL_ELF) $(SHELL_BIN)

.PHONY: FORCE
FORCE:

.PHONY: cap_test
cap_test:
	cd $(CAP_TEST_DIR) && $(CARGO) build --$(PROFILE)
	$(OBJCOPY) -O binary --set-section-flags .bss=alloc,load,contents \
		$(CAP_TEST_DIR)/target/$(CAP_TEST_TARGET)/$(PROFILE)/task_a $(USERLAND_DIR)/cap_test_a.bin
	$(OBJCOPY) -O binary --set-section-flags .bss=alloc,load,contents \
		$(CAP_TEST_DIR)/target/$(CAP_TEST_TARGET)/$(PROFILE)/task_b $(USERLAND_DIR)/cap_test_b.bin
	$(OBJCOPY) -O binary --set-section-flags .bss=alloc,load,contents \
		$(CAP_TEST_DIR)/target/$(CAP_TEST_TARGET)/$(PROFILE)/mem_test_a $(USERLAND_DIR)/mem_test_a.bin
	$(OBJCOPY) -O binary --set-section-flags .bss=alloc,load,contents \
		$(CAP_TEST_DIR)/target/$(CAP_TEST_TARGET)/$(PROFILE)/mem_test_b $(USERLAND_DIR)/mem_test_b.bin
	$(OBJCOPY) -O binary --set-section-flags .bss=alloc,load,contents \
		$(CAP_TEST_DIR)/target/$(CAP_TEST_TARGET)/$(PROFILE)/storage_client_test $(USERLAND_DIR)/storage_client_test.bin
	$(OBJCOPY) -O binary --set-section-flags .bss=alloc,load,contents \
		$(CAP_TEST_DIR)/target/$(CAP_TEST_TARGET)/$(PROFILE)/fs_client_test $(USERLAND_DIR)/fs_client_test.bin
	$(OBJCOPY) -O binary --set-section-flags .bss=alloc,load,contents \
		$(CAP_TEST_DIR)/target/$(CAP_TEST_TARGET)/$(PROFILE)/console_input_test $(USERLAND_DIR)/console_input_test.bin
	$(OBJCOPY) -O binary --set-section-flags .bss=alloc,load,contents \
		$(CAP_TEST_DIR)/target/$(CAP_TEST_TARGET)/$(PROFILE)/raw_input_test $(USERLAND_DIR)/raw_input_test.bin
	$(OBJCOPY) -O binary --set-section-flags .bss=alloc,load,contents \
		$(CAP_TEST_DIR)/target/$(CAP_TEST_TARGET)/$(PROFILE)/editor_input_test $(USERLAND_DIR)/editor_input_test.bin
	$(OBJCOPY) -O binary --set-section-flags .bss=alloc,load,contents \
		$(CAP_TEST_DIR)/target/$(CAP_TEST_TARGET)/$(PROFILE)/loaded_program $(USERLAND_DIR)/loaded_program.bin
	$(OBJCOPY) -O binary --set-section-flags .bss=alloc,load,contents \
		$(CAP_TEST_DIR)/target/$(CAP_TEST_TARGET)/$(PROFILE)/reboot_test $(USERLAND_DIR)/reboot_test.bin
	$(OBJCOPY) -O binary --set-section-flags .bss=alloc,load,contents \
		$(CAP_TEST_DIR)/target/$(CAP_TEST_TARGET)/$(PROFILE)/nic_test $(USERLAND_DIR)/nic_test.bin
	$(OBJCOPY) -O binary --set-section-flags .bss=alloc,load,contents \
		$(CAP_TEST_DIR)/target/$(CAP_TEST_TARGET)/$(PROFILE)/http_client_test $(USERLAND_DIR)/http_client_test.bin

.PHONY: iso
iso: kernel userland
	$(MKDIR) $(GRUB_PATH)
	$(CP) $(KERNEL_BIN) $(BOOT_PATH)/pcern.elf
	$(foreach bin,$(PROD_USERLAND_BINS),$(CP) $(bin) $(BOOT_PATH)/$(notdir $(bin));)
	$(CP) $(CFG) $(GRUB_PATH)
	grub-mkrescue -o $(ISO) $(ISO_PATH)

.PHONY: kernel-test
kernel-test:
	cd $(KERNEL_DIR) && $(CARGO) build --$(PROFILE) --features test_harness
	grub-file --is-x86-multiboot $(KERNEL_BIN)

.PHONY: iso-test
iso-test: kernel-test userland cap_test
	$(MKDIR) $(GRUB_TEST_PATH)
	$(CP) $(KERNEL_BIN) $(BOOT_TEST_PATH)/pcern.elf
	$(CP) $(CONSOLE_SERVER_BIN) $(BOOT_TEST_PATH)/console_server.bin
	$(CP) $(NAMESERVICE_BIN) $(BOOT_TEST_PATH)/nameservice.bin
	$(CP) $(STORAGE_ATA_BIN) $(BOOT_TEST_PATH)/storage_ata.bin
	$(CP) $(FS_FAT32_BIN) $(BOOT_TEST_PATH)/fs_fat32.bin
	$(CP) $(USERLAND_DIR)/cap_test_a.bin $(BOOT_TEST_PATH)/cap_test_a.bin
	$(CP) $(USERLAND_DIR)/cap_test_b.bin $(BOOT_TEST_PATH)/cap_test_b.bin
	$(CP) $(USERLAND_DIR)/mem_test_a.bin $(BOOT_TEST_PATH)/mem_test_a.bin
	$(CP) $(USERLAND_DIR)/mem_test_b.bin $(BOOT_TEST_PATH)/mem_test_b.bin
	$(CP) $(USERLAND_DIR)/fs_client_test.bin $(BOOT_TEST_PATH)/fs_client_test.bin
	$(CP) $(CFG_TEST) $(GRUB_TEST_PATH)/grub.cfg
	grub-mkrescue -o $(ISO_TEST) $(ISO_TEST_PATH)

.PHONY: kernel-keytest
kernel-keytest:
	cd $(KERNEL_DIR) && $(CARGO) build --$(PROFILE) --features keyboard_test
	grub-file --is-x86-multiboot $(KERNEL_BIN)

.PHONY: iso-keytest
iso-keytest: kernel-keytest userland cap_test
	$(MKDIR) $(GRUB_KEYTEST_PATH)
	$(CP) $(KERNEL_BIN) $(BOOT_KEYTEST_PATH)/pcern.elf
	$(CP) $(CONSOLE_SERVER_BIN) $(BOOT_KEYTEST_PATH)/console_server.bin
	$(CP) $(NAMESERVICE_BIN) $(BOOT_KEYTEST_PATH)/nameservice.bin
	$(CP) $(STORAGE_ATA_BIN) $(BOOT_KEYTEST_PATH)/storage_ata.bin
	$(CP) $(FS_FAT32_BIN) $(BOOT_KEYTEST_PATH)/fs_fat32.bin
	$(CP) $(USERLAND_DIR)/console_input_test.bin $(BOOT_KEYTEST_PATH)/console_input_test.bin
	$(CP) $(CFG_KEYTEST) $(GRUB_KEYTEST_PATH)/grub.cfg
	grub-mkrescue -o $(ISO_KEYTEST) $(ISO_KEYTEST_PATH)

.PHONY: test-keyboard
test-keyboard: iso-keytest
	./scripts/test/run_console_input_test.sh $(ISO_KEYTEST)

.PHONY: kernel-rawtest
kernel-rawtest:
	cd $(KERNEL_DIR) && $(CARGO) build --$(PROFILE) --features raw_input_test
	grub-file --is-x86-multiboot $(KERNEL_BIN)

.PHONY: iso-rawtest
iso-rawtest: kernel-rawtest userland cap_test
	$(MKDIR) $(GRUB_RAWTEST_PATH)
	$(CP) $(KERNEL_BIN) $(BOOT_RAWTEST_PATH)/pcern.elf
	$(CP) $(CONSOLE_SERVER_BIN) $(BOOT_RAWTEST_PATH)/console_server.bin
	$(CP) $(NAMESERVICE_BIN) $(BOOT_RAWTEST_PATH)/nameservice.bin
	$(CP) $(STORAGE_ATA_BIN) $(BOOT_RAWTEST_PATH)/storage_ata.bin
	$(CP) $(FS_FAT32_BIN) $(BOOT_RAWTEST_PATH)/fs_fat32.bin
	$(CP) $(USERLAND_DIR)/raw_input_test.bin $(BOOT_RAWTEST_PATH)/raw_input_test.bin
	$(CP) $(CFG_RAWTEST) $(GRUB_RAWTEST_PATH)/grub.cfg
	grub-mkrescue -o $(ISO_RAWTEST) $(ISO_RAWTEST_PATH)

.PHONY: test-raw-input
test-raw-input: iso-rawtest
	./scripts/test/run_raw_input_test.sh $(ISO_RAWTEST)

.PHONY: kernel-editortest
kernel-editortest:
	cd $(KERNEL_DIR) && $(CARGO) build --$(PROFILE) --features editor_test
	grub-file --is-x86-multiboot $(KERNEL_BIN)

.PHONY: iso-editortest
iso-editortest: kernel-editortest userland cap_test
	$(MKDIR) $(GRUB_EDITORTEST_PATH)
	$(CP) $(KERNEL_BIN) $(BOOT_EDITORTEST_PATH)/pcern.elf
	$(CP) $(CONSOLE_SERVER_BIN) $(BOOT_EDITORTEST_PATH)/console_server.bin
	$(CP) $(NAMESERVICE_BIN) $(BOOT_EDITORTEST_PATH)/nameservice.bin
	$(CP) $(STORAGE_ATA_BIN) $(BOOT_EDITORTEST_PATH)/storage_ata.bin
	$(CP) $(FS_FAT32_BIN) $(BOOT_EDITORTEST_PATH)/fs_fat32.bin
	$(CP) $(USERLAND_DIR)/editor_input_test.bin $(BOOT_EDITORTEST_PATH)/editor_input_test.bin
	$(CP) $(CFG_EDITORTEST) $(GRUB_EDITORTEST_PATH)/grub.cfg
	grub-mkrescue -o $(ISO_EDITORTEST) $(ISO_EDITORTEST_PATH)

.PHONY: test-editor
test-editor: iso-editortest test-fat32-image
	./scripts/test/run_editor_test.sh $(ISO_EDITORTEST) $(TEST_FAT32_IMG)

.PHONY: kernel-reboottest
kernel-reboottest:
	cd $(KERNEL_DIR) && $(CARGO) build --$(PROFILE) --features reboot_test
	grub-file --is-x86-multiboot $(KERNEL_BIN)

.PHONY: iso-reboottest
iso-reboottest: kernel-reboottest userland cap_test
	$(MKDIR) $(GRUB_REBOOTTEST_PATH)
	$(CP) $(KERNEL_BIN) $(BOOT_REBOOTTEST_PATH)/pcern.elf
	$(CP) $(CONSOLE_SERVER_BIN) $(BOOT_REBOOTTEST_PATH)/console_server.bin
	$(CP) $(NAMESERVICE_BIN) $(BOOT_REBOOTTEST_PATH)/nameservice.bin
	$(CP) $(STORAGE_ATA_BIN) $(BOOT_REBOOTTEST_PATH)/storage_ata.bin
	$(CP) $(FS_FAT32_BIN) $(BOOT_REBOOTTEST_PATH)/fs_fat32.bin
	$(CP) $(USERLAND_DIR)/reboot_test.bin $(BOOT_REBOOTTEST_PATH)/reboot_test.bin
	$(CP) $(CFG_REBOOTTEST) $(GRUB_REBOOTTEST_PATH)/grub.cfg
	grub-mkrescue -o $(ISO_REBOOTTEST) $(ISO_REBOOTTEST_PATH)

.PHONY: test-reboot
test-reboot: iso-reboottest
	./scripts/test/run_reboot_test.sh $(ISO_REBOOTTEST)

.PHONY: kernel-nictest
kernel-nictest:
	cd $(KERNEL_DIR) && $(CARGO) build --$(PROFILE) --features nic_test
	grub-file --is-x86-multiboot $(KERNEL_BIN)

.PHONY: iso-nictest
iso-nictest: kernel-nictest userland cap_test
	$(MKDIR) $(GRUB_NICTEST_PATH)
	$(CP) $(KERNEL_BIN) $(BOOT_NICTEST_PATH)/pcern.elf
	$(CP) $(CONSOLE_SERVER_BIN) $(BOOT_NICTEST_PATH)/console_server.bin
	$(CP) $(NAMESERVICE_BIN) $(BOOT_NICTEST_PATH)/nameservice.bin
	$(CP) $(STORAGE_ATA_BIN) $(BOOT_NICTEST_PATH)/storage_ata.bin
	$(CP) $(FS_FAT32_BIN) $(BOOT_NICTEST_PATH)/fs_fat32.bin
	$(CP) $(NET_RTL8139_BIN) $(BOOT_NICTEST_PATH)/net_rtl8139.bin
	$(CP) $(USERLAND_DIR)/nic_test.bin $(BOOT_NICTEST_PATH)/nic_test.bin
	$(CP) $(CFG_NICTEST) $(GRUB_NICTEST_PATH)/grub.cfg
	grub-mkrescue -o $(ISO_NICTEST) $(ISO_NICTEST_PATH)

.PHONY: test-nic
test-nic: iso-nictest
	./scripts/test/run_nic_test.sh $(ISO_NICTEST)

.PHONY: kernel-arptest
kernel-arptest:
	cd $(KERNEL_DIR) && $(CARGO) build --$(PROFILE) --features arp_icmp_test
	grub-file --is-x86-multiboot $(KERNEL_BIN)

.PHONY: iso-arptest
iso-arptest: kernel-arptest userland
	$(MKDIR) $(GRUB_ARPTEST_PATH)
	$(CP) $(KERNEL_BIN) $(BOOT_ARPTEST_PATH)/pcern.elf
	$(CP) $(CONSOLE_SERVER_BIN) $(BOOT_ARPTEST_PATH)/console_server.bin
	$(CP) $(NAMESERVICE_BIN) $(BOOT_ARPTEST_PATH)/nameservice.bin
	$(CP) $(STORAGE_ATA_BIN) $(BOOT_ARPTEST_PATH)/storage_ata.bin
	$(CP) $(FS_FAT32_BIN) $(BOOT_ARPTEST_PATH)/fs_fat32.bin
	$(CP) $(NET_RTL8139_BIN) $(BOOT_ARPTEST_PATH)/net_rtl8139.bin
	$(CP) $(NETSTACK_BIN) $(BOOT_ARPTEST_PATH)/netstack.bin
	$(CP) $(CFG_ARPTEST) $(GRUB_ARPTEST_PATH)/grub.cfg
	grub-mkrescue -o $(ISO_ARPTEST) $(ISO_ARPTEST_PATH)

.PHONY: test-arp
test-arp: iso-arptest
	./scripts/test/run_arp_icmp_test.sh $(ISO_ARPTEST)

.PHONY: kernel-tcptest
kernel-tcptest:
	cd $(KERNEL_DIR) && $(CARGO) build --$(PROFILE) --features tcp_test
	grub-file --is-x86-multiboot $(KERNEL_BIN)

.PHONY: iso-tcptest
iso-tcptest: kernel-tcptest userland cap_test
	$(MKDIR) $(GRUB_TCPTEST_PATH)
	$(CP) $(KERNEL_BIN) $(BOOT_TCPTEST_PATH)/pcern.elf
	$(CP) $(CONSOLE_SERVER_BIN) $(BOOT_TCPTEST_PATH)/console_server.bin
	$(CP) $(NAMESERVICE_BIN) $(BOOT_TCPTEST_PATH)/nameservice.bin
	$(CP) $(STORAGE_ATA_BIN) $(BOOT_TCPTEST_PATH)/storage_ata.bin
	$(CP) $(FS_FAT32_BIN) $(BOOT_TCPTEST_PATH)/fs_fat32.bin
	$(CP) $(USERLAND_DIR)/http_client_test.bin $(BOOT_TCPTEST_PATH)/http_client_test.bin
	$(CP) $(NET_RTL8139_BIN) $(BOOT_TCPTEST_PATH)/net_rtl8139.bin
	$(CP) $(NETSTACK_BIN) $(BOOT_TCPTEST_PATH)/netstack.bin
	$(CP) $(CFG_TCPTEST) $(GRUB_TCPTEST_PATH)/grub.cfg
	grub-mkrescue -o $(ISO_TCPTEST) $(ISO_TCPTEST_PATH)

.PHONY: test-tcp
test-tcp: iso-tcptest
	./scripts/test/run_tcp_test.sh $(ISO_TCPTEST)

.PHONY: test
test: iso-test test-fat32-image
	./scripts/test/run_tests.sh $(ISO_TEST) $(TEST_FAT32_IMG)
	$(MAKE) test-keyboard
	$(MAKE) test-raw-input
	$(MAKE) test-editor
	$(MAKE) test-reboot
	$(MAKE) test-nic
	$(MAKE) test-arp
	$(MAKE) test-tcp
	$(MAKE) test-disk-boot

.PHONY: test-fat32-image
test-fat32-image: cap_test
	$(RM) $(TEST_FAT32_IMG)
	dd if=/dev/zero of=$(TEST_FAT32_IMG) bs=1M count=64 status=none
	$(MTOOLS_MFORMAT) -i $(TEST_FAT32_IMG) -F -v PCERNFS ::
	$(MTOOLS_MCOPY) -i $(TEST_FAT32_IMG) $(TESTDATA_DIR)/HELLO.TXT ::HELLO.TXT
	$(MTOOLS_MCOPY) -i $(TEST_FAT32_IMG) $(TESTDATA_DIR)/BIG.TXT ::BIG.TXT
	$(MTOOLS_MCOPY) -i $(TEST_FAT32_IMG) $(USERLAND_DIR)/loaded_program.bin ::LOADED.BIN

.PHONY: run
run: iso
	qemu-system-i386 -cdrom $(ISO) -serial stdio

# Checkpoint U: builds $(DISK_IMG), a real installed FAT32 boot disk --
# see the variable block above for why it needs an MBR partition table
# GRUB can embed core.img into, unlike the plain "superfloppy" FAT32 test
# image `test-fat32-image` builds. Every payload binary lands as an
# ordinary root-level file on the FAT32 partition via mtools, through the
# exact same 8.3-name/root-directory-only interface fs_fat32's own
# runtime read/write protocol already supports -- nothing about *how*
# these files get there is special-cased for boot; a real update
# mechanism (ZephyrLite's own Checkpoint Z, not yet built) would overwrite
# them the same way. GRUB's own bootstrap machinery (core.img, embedded
# straight into the raw sectors between the MBR and this partition) is
# written once by grub-mkstandalone/grub-bios-setup below and never
# touched by fs_fat32 at runtime -- it doesn't even live inside the FAT32
# filesystem fs_fat32 can see.
#
# grub-bios-setup itself needs `sudo`: it always resolves what device
# backs its own `-d` source directory (to record a fallback GRUB `root`
# environment variable, even though our grub-disk.cfg immediately
# overrides it) by opening that device's raw node, which needs root
# regardless of the fact that DEST is a plain file we own -- the exact
# same requirement a real `grub-install` has on real hardware. Every
# other step here (partitioning, formatting, mtools) works unprivileged.
.PHONY: disk
disk: kernel userland
	$(RM) $(DISK_BUILD_DIR) $(DISK_FAT_IMG) $(DISK_IMG)
	$(MKDIR) $(DISK_BUILD_DIR)
	grub-mkstandalone -O i386-pc -o $(DISK_BUILD_DIR)/core.img \
		--install-modules="$(GRUB_I386_PC_MODULES)" \
		--modules="$(GRUB_I386_PC_MODULES)" \
		--fonts="" --locales="" --themes="" \
		"boot/grub/grub.cfg=$(CFG_DISK)"
	$(CP) /usr/lib/grub/i386-pc/boot.img $(DISK_BUILD_DIR)/boot.img
	truncate -s $(DISK_SIZE_MB)M $(DISK_IMG)
	printf 'label: dos\nunit: sectors\nstart=%s, type=c\n' $(DISK_PART_START_SECTOR) | sfdisk $(DISK_IMG)
	mkfs.vfat -F 32 -n ZLBOOT -C $(DISK_FAT_IMG) \
		$$(( ($(DISK_SIZE_MB) * 2048 - $(DISK_PART_START_SECTOR)) / 2 ))
	for pair in $(PROD_USERLAND_DISK_FILES); do \
		src="$${pair%%:*}"; dst="$${pair#*:}"; \
		$(MTOOLS_MCOPY) -i $(DISK_FAT_IMG) "$$src" "::$$dst"; \
	done
	$(MTOOLS_MCOPY) -i $(DISK_FAT_IMG) $(KERNEL_BIN) ::PCERN.ELF
	dd if=$(DISK_FAT_IMG) of=$(DISK_IMG) bs=512 seek=$(DISK_PART_START_SECTOR) conv=notrunc,sparse status=none
	printf '(hd0)\t%s\n' $(DISK_IMG) > $(DISK_BUILD_DIR)/device.map
	sudo $(GRUB_BIOS_SETUP) -d $(DISK_BUILD_DIR) -b boot.img -c core.img \
		-m $(DISK_BUILD_DIR)/device.map $(DISK_IMG)

.PHONY: run-disk
run-disk: disk
	qemu-system-i386 -drive file=$(DISK_IMG),if=ide,index=0,format=raw -boot c -serial stdio

.PHONY: test-disk-boot
test-disk-boot: disk
	./scripts/test/run_disk_boot_test.sh $(DISK_IMG)

.PHONY: clean
clean:
	cd $(KERNEL_DIR) && $(CARGO) clean
	cd $(CONSOLE_SERVER_DIR) && $(CARGO) clean
	cd $(NAMESERVICE_DIR) && $(CARGO) clean
	cd $(STORAGE_ATA_DIR) && $(CARGO) clean
	cd $(FS_FAT32_DIR) && $(CARGO) clean
	cd $(NET_RTL8139_DIR) && $(CARGO) clean
	cd $(NETSTACK_DIR) && $(CARGO) clean
	cd $(SHELL_DIR) && $(CARGO) clean
	cd $(CAP_TEST_DIR) && $(CARGO) clean
	$(RM) $(ISO_PATH) $(ISO) $(ISO_TEST_PATH) $(ISO_TEST) $(ISO_KEYTEST_PATH) $(ISO_KEYTEST) $(ISO_RAWTEST_PATH) $(ISO_RAWTEST) $(ISO_EDITORTEST_PATH) $(ISO_EDITORTEST) $(ISO_REBOOTTEST_PATH) $(ISO_REBOOTTEST) $(ISO_NICTEST_PATH) $(ISO_NICTEST) $(ISO_ARPTEST_PATH) $(ISO_ARPTEST) $(ISO_TCPTEST_PATH) $(ISO_TCPTEST) $(USERLAND_DIR)/*.bin $(TEST_FAT32_IMG) $(DISK_BUILD_DIR) $(DISK_FAT_IMG) $(DISK_IMG)
