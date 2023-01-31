CP := cp
RM := rm -rf
MKDIR := mkdir -pv
COMPILE := i386-elf-gcc -std=gnu99 -ffreestanding -g
LINK := i386-elf-gcc -ffreestanding -nostdlib -g -T

BIN = pcern.elf
CFG = grub.cfg
ISO_PATH := iso
BOOT_PATH := $(ISO_PATH)/boot
GRUB_PATH := $(BOOT_PATH)/grub

.PHONY: all
all: bootloader kernel linker iso
	@echo Make has completed.

bootloader: boot.s
	$(COMPILE) -c boot.s -o boot.o

kernel: kernel.c
	$(COMPILE) -c kernel.c -o kernel.o

linker: linker.ld boot.o kernel.o
	$(LINK) linker.ld boot.o kernel.o -o pcern.elf -lgcc

iso: pcern.elf
	$(MKDIR) $(GRUB_PATH)
	$(CP) $(BIN) $(BOOT_PATH)
	$(CP) $(CFG) $(GRUB_PATH)
	grub-file --is-x86-multiboot $(BOOT_PATH)/$(BIN)
	grub-mkrescue $(ISO_PATH) -o pcern-i386.iso

.PHONY: clean
clean:
	$(RM) *.o $(BIN) *iso
