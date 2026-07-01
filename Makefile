CARGO := cargo
PROFILE := release
TARGET := i686-pcern
KERNEL_BIN := target/$(TARGET)/$(PROFILE)/pcern

NASM := nasm
USERLAND_DIR := userland
USERLAND_BINS := $(USERLAND_DIR)/ping.bin $(USERLAND_DIR)/pong.bin

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
userland: $(USERLAND_BINS)

$(USERLAND_DIR)/%.bin: $(USERLAND_DIR)/%.asm
	$(NASM) -f bin $< -o $@

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
	$(RM) $(ISO_PATH) $(ISO) $(USERLAND_DIR)/*.bin
