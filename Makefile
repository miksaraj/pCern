CARGO := cargo
PROFILE := release
TARGET := i686-pcern
KERNEL_BIN := target/$(TARGET)/$(PROFILE)/pcern

OBJCOPY := objcopy
USERLAND_DIR := userland

CONSOLE_SERVER_DIR := $(USERLAND_DIR)/console_server
CONSOLE_SERVER_TARGET := i686-pcern-user
CONSOLE_SERVER_ELF := $(CONSOLE_SERVER_DIR)/target/$(CONSOLE_SERVER_TARGET)/$(PROFILE)/console_server
CONSOLE_SERVER_BIN := $(USERLAND_DIR)/console_server.bin

NAMESERVICE_DIR := $(USERLAND_DIR)/nameservice
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

# Test harness (see `make test`): a separate kernel build (--features
# test_harness) + grub config + ISO, so the production `iso`/`kernel`
# targets above are completely untouched by any of this.
CFG_TEST := grub-test.cfg
ISO_TEST_PATH := iso-test
BOOT_TEST_PATH := $(ISO_TEST_PATH)/boot
GRUB_TEST_PATH := $(BOOT_TEST_PATH)/grub
ISO_TEST := pcern-test-i386.iso

# Checkpoint L's keyboard-input test harness: its own standalone kernel
# build (--features keyboard_test) + grub config + ISO, separate from
# both the production and the shared iso-test builds above -- see
# console_input_test.rs's doc comment for why this fixture can't share
# either of those.
CFG_KEYTEST := grub-keytest.cfg
ISO_KEYTEST_PATH := iso-keytest
BOOT_KEYTEST_PATH := $(ISO_KEYTEST_PATH)/boot
GRUB_KEYTEST_PATH := $(BOOT_KEYTEST_PATH)/grub
ISO_KEYTEST := pcern-keytest-i386.iso

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
userland: $(CONSOLE_SERVER_BIN) $(NAMESERVICE_BIN) $(STORAGE_ATA_BIN) $(FS_FAT32_BIN)

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
	$(OBJCOPY) -O binary --set-section-flags .bss=alloc,load,contents \
		$(CAP_TEST_DIR)/target/$(CAP_TEST_TARGET)/$(PROFILE)/console_input_test $(USERLAND_DIR)/console_input_test.bin
	$(OBJCOPY) -O binary --set-section-flags .bss=alloc,load,contents \
		$(CAP_TEST_DIR)/target/$(CAP_TEST_TARGET)/$(PROFILE)/loaded_program $(USERLAND_DIR)/loaded_program.bin

.PHONY: iso
iso: kernel userland
	$(MKDIR) $(GRUB_PATH)
	$(CP) $(KERNEL_BIN) $(BOOT_PATH)/pcern.elf
	$(CP) $(CONSOLE_SERVER_BIN) $(BOOT_PATH)/console_server.bin
	$(CP) $(NAMESERVICE_BIN) $(BOOT_PATH)/nameservice.bin
	$(CP) $(STORAGE_ATA_BIN) $(BOOT_PATH)/storage_ata.bin
	$(CP) $(FS_FAT32_BIN) $(BOOT_PATH)/fs_fat32.bin
	$(CP) $(CFG) $(GRUB_PATH)
	grub-mkrescue -o $(ISO) $(ISO_PATH)

.PHONY: kernel-test
kernel-test:
	$(CARGO) build --$(PROFILE) --features test_harness
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
	$(CARGO) build --$(PROFILE) --features keyboard_test
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
	./run_console_input_test.sh $(ISO_KEYTEST)

.PHONY: test
test: iso-test test-fat32-image
	./run_tests.sh $(ISO_TEST) $(TEST_FAT32_IMG)
	$(MAKE) test-keyboard

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

.PHONY: clean
clean:
	$(CARGO) clean
	cd $(CONSOLE_SERVER_DIR) && $(CARGO) clean
	cd $(NAMESERVICE_DIR) && $(CARGO) clean
	cd $(STORAGE_ATA_DIR) && $(CARGO) clean
	cd $(FS_FAT32_DIR) && $(CARGO) clean
	cd $(CAP_TEST_DIR) && $(CARGO) clean
	$(RM) $(ISO_PATH) $(ISO) $(ISO_TEST_PATH) $(ISO_TEST) $(ISO_KEYTEST_PATH) $(ISO_KEYTEST) $(USERLAND_DIR)/*.bin $(TEST_FAT32_IMG)
