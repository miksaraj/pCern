CARGO := cargo
PROFILE := release
TARGET := i686-pcern
KERNEL_BIN := target/$(TARGET)/$(PROFILE)/pcern

NASM := nasm
OBJCOPY := objcopy
USERLAND_DIR := userland
USERLAND_BINS := $(USERLAND_DIR)/ping.bin $(USERLAND_DIR)/pong.bin

CONSOLE_SERVER_DIR := $(USERLAND_DIR)/console_server
CONSOLE_SERVER_TARGET := i686-pcern-user
CONSOLE_SERVER_ELF := $(CONSOLE_SERVER_DIR)/target/$(CONSOLE_SERVER_TARGET)/$(PROFILE)/console_server
CONSOLE_SERVER_BIN := $(USERLAND_DIR)/console_server.bin

NAMESERVICE_DIR := $(USERLAND_DIR)/nameservice
NAMESERVICE_TARGET := i686-pcern-user
NAMESERVICE_ELF := $(NAMESERVICE_DIR)/target/$(NAMESERVICE_TARGET)/$(PROFILE)/nameservice
NAMESERVICE_BIN := $(USERLAND_DIR)/nameservice.bin

# Checkpoint F/G test fixtures (see userland/cap_test) -- not part of the
# default `iso`/`userland` build, same as driver_test.asm/irq_test.asm;
# built on demand via `make cap_test` for temporary verification only.
CAP_TEST_DIR := $(USERLAND_DIR)/cap_test
CAP_TEST_TARGET := i686-pcern-user

STORAGE_ATA_DIR := $(USERLAND_DIR)/storage_ata
STORAGE_ATA_TARGET := i686-pcern-user
STORAGE_ATA_ELF := $(STORAGE_ATA_DIR)/target/$(STORAGE_ATA_TARGET)/$(PROFILE)/storage_ata
STORAGE_ATA_BIN := $(USERLAND_DIR)/storage_ata.bin

FS_FAT32_DIR := $(USERLAND_DIR)/fs_fat32
FS_FAT32_TARGET := i686-pcern-user
FS_FAT32_ELF := $(FS_FAT32_DIR)/target/$(FS_FAT32_TARGET)/$(PROFILE)/fs_fat32
FS_FAT32_BIN := $(USERLAND_DIR)/fs_fat32.bin

CP := cp
RM := rm -rf
MKDIR := mkdir -pv

CFG := grub.cfg
ISO_PATH := iso
BOOT_PATH := $(ISO_PATH)/boot
GRUB_PATH := $(BOOT_PATH)/grub
ISO := pcern-i386.iso

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

.PHONY: all
all: iso
	@echo Make has completed.

.PHONY: kernel
kernel:
	$(CARGO) build --$(PROFILE)
	grub-file --is-x86-multiboot $(KERNEL_BIN)

.PHONY: userland
userland: $(USERLAND_BINS) $(CONSOLE_SERVER_BIN) $(NAMESERVICE_BIN) $(STORAGE_ATA_BIN) $(FS_FAT32_BIN)

$(USERLAND_DIR)/%.bin: $(USERLAND_DIR)/%.asm
	$(NASM) -f bin $< -o $@

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

.PHONY: iso
iso: kernel userland
	$(MKDIR) $(GRUB_PATH)
	$(CP) $(KERNEL_BIN) $(BOOT_PATH)/pcern.elf
	$(CP) $(USERLAND_DIR)/ping.bin $(BOOT_PATH)/ping.bin
	$(CP) $(USERLAND_DIR)/pong.bin $(BOOT_PATH)/pong.bin
	$(CP) $(CONSOLE_SERVER_BIN) $(BOOT_PATH)/console_server.bin
	$(CP) $(NAMESERVICE_BIN) $(BOOT_PATH)/nameservice.bin
	$(CP) $(STORAGE_ATA_BIN) $(BOOT_PATH)/storage_ata.bin
	$(CP) $(FS_FAT32_BIN) $(BOOT_PATH)/fs_fat32.bin
	$(CP) $(CFG) $(GRUB_PATH)
	grub-mkrescue -o $(ISO) $(ISO_PATH)

.PHONY: test-fat32-image
test-fat32-image:
	$(RM) $(TEST_FAT32_IMG)
	dd if=/dev/zero of=$(TEST_FAT32_IMG) bs=1M count=64 status=none
	$(MTOOLS_MFORMAT) -i $(TEST_FAT32_IMG) -F -v PCERNFS ::
	$(MTOOLS_MCOPY) -i $(TEST_FAT32_IMG) $(TESTDATA_DIR)/HELLO.TXT ::HELLO.TXT
	$(MTOOLS_MCOPY) -i $(TEST_FAT32_IMG) $(TESTDATA_DIR)/BIG.TXT ::BIG.TXT

.PHONY: run
run: iso
	qemu-system-i386 -cdrom $(ISO) -serial stdio

.PHONY: clean
clean:
	$(CARGO) clean
	cd $(CONSOLE_SERVER_DIR) && $(CARGO) clean
	cd $(NAMESERVICE_DIR) && $(CARGO) clean
	cd $(STORAGE_ATA_DIR) && $(CARGO) clean
	cd $(FS_FAT32_DIR) && $(CARGO) clean
	cd $(CAP_TEST_DIR) && $(CARGO) clean
	$(RM) $(ISO_PATH) $(ISO) $(USERLAND_DIR)/*.bin $(TEST_FAT32_IMG)
