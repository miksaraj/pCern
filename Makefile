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

.PHONY: iso
iso: kernel userland
	$(MKDIR) $(GRUB_PATH)
	$(CP) $(KERNEL_BIN) $(BOOT_PATH)/pcern.elf
	$(CP) $(USERLAND_DIR)/ping.bin $(BOOT_PATH)/ping.bin
	$(CP) $(USERLAND_DIR)/pong.bin $(BOOT_PATH)/pong.bin
	$(CP) $(CFG) $(GRUB_PATH)
	grub-mkrescue -o $(ISO) $(ISO_PATH)

.PHONY: run
run: iso
	qemu-system-i386 -cdrom $(ISO) -serial stdio

.PHONY: clean
clean:
	$(CARGO) clean
	cd $(CONSOLE_SERVER_DIR) && $(CARGO) clean
	$(RM) $(ISO_PATH) $(ISO) $(USERLAND_DIR)/*.bin
