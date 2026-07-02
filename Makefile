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

# Checkpoint F/G test fixtures (see userland/cap_test) -- not part of the
# default `iso`/`userland` build, same as driver_test.asm/irq_test.asm;
# built on demand via `make cap_test` for temporary verification only.
CAP_TEST_DIR := $(USERLAND_DIR)/cap_test
CAP_TEST_TARGET := i686-pcern-user

CP := cp
RM := rm -rf
MKDIR := mkdir -pv

CFG := grub.cfg
ISO_PATH := iso
BOOT_PATH := $(ISO_PATH)/boot
GRUB_PATH := $(BOOT_PATH)/grub
ISO := pcern-i386.iso

.PHONY: all
all: iso
	@echo Make has completed.

.PHONY: kernel
kernel:
	$(CARGO) build --$(PROFILE)
	grub-file --is-x86-multiboot $(KERNEL_BIN)

.PHONY: userland
userland: $(USERLAND_BINS) $(CONSOLE_SERVER_BIN)

$(USERLAND_DIR)/%.bin: $(USERLAND_DIR)/%.asm
	$(NASM) -f bin $< -o $@

$(CONSOLE_SERVER_BIN): FORCE
	cd $(CONSOLE_SERVER_DIR) && $(CARGO) build --$(PROFILE)
	$(OBJCOPY) -O binary --set-section-flags .bss=alloc,load,contents $(CONSOLE_SERVER_ELF) $(CONSOLE_SERVER_BIN)

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

.PHONY: iso
iso: kernel userland
	$(MKDIR) $(GRUB_PATH)
	$(CP) $(KERNEL_BIN) $(BOOT_PATH)/pcern.elf
	$(CP) $(USERLAND_DIR)/ping.bin $(BOOT_PATH)/ping.bin
	$(CP) $(USERLAND_DIR)/pong.bin $(BOOT_PATH)/pong.bin
	$(CP) $(CONSOLE_SERVER_BIN) $(BOOT_PATH)/console_server.bin
	$(CP) $(CFG) $(GRUB_PATH)
	grub-mkrescue -o $(ISO) $(ISO_PATH)

.PHONY: run
run: iso
	qemu-system-i386 -cdrom $(ISO) -serial stdio

.PHONY: clean
clean:
	$(CARGO) clean
	cd $(CONSOLE_SERVER_DIR) && $(CARGO) clean
	cd $(CAP_TEST_DIR) && $(CARGO) clean
	$(RM) $(ISO_PATH) $(ISO) $(USERLAND_DIR)/*.bin
