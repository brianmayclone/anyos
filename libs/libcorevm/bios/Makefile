NASM ?= nasm
OUT   = bios.bin

$(OUT): bios.asm post.asm ivt.asm bda.asm strings.asm e820.asm \
        pic_pit.asm serial.asm video_init.asm pci.asm ide.asm \
        int10h.asm int13h.asm int15h.asm int16h.asm int19h.asm \
        int1ah.asm int_misc.asm selftest.asm boot.asm
	$(NASM) -f bin -o $@ $<
	@size=$$(wc -c < $@); \
	if [ $$size -ne 65536 ]; then \
		echo "ERROR: BIOS binary is $$size bytes (expected 65536)"; \
		rm -f $@; \
		exit 1; \
	fi
	@echo "OK: $@ is 65536 bytes"

clean:
	rm -f $(OUT)

.PHONY: clean
