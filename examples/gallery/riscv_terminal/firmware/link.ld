MEMORY { ram (rwx) : ORIGIN = 0, LENGTH = 32K }
SECTIONS {
    .text : { *(.text.start) *(.text .text.*) } > ram
    .rodata : { *(.rodata .rodata.* .srodata .srodata.*) } > ram
    .data : { *(.data .data.* .sdata .sdata.*) } > ram
    /* Loaded zeros at power-on; a reset does not erase physical RAM. */
    .persist : ALIGN(4) { KEEP(*(.persist)) } > ram
    _image_end = .;
    .bss (NOLOAD) : ALIGN(4) {
        _bss_start = .;
        *(.bss .bss.* .sbss .sbss.* COMMON)
        . = ALIGN(4);
        _bss_end = .;
    } > ram
    _stack_top = ORIGIN(ram) + LENGTH(ram);
    ASSERT(_bss_end < _stack_top - 4096, "leave scratch RAM and stack space")
}
