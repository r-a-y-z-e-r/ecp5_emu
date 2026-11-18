/* Code, data and stack live in SDRAM. The test starts above them at 1 MB. */
MEMORY { ram (rwx) : ORIGIN = 0x40000000, LENGTH = 64K }
SECTIONS {
    .text : { *(.text.start) *(.text .text.*) } > ram
    .rodata : { *(.rodata .rodata.* .srodata .srodata.*) } > ram
    .data : { *(.data .data.* .sdata .sdata.*) } > ram
    .bss (NOLOAD) : ALIGN(4) {
        _bss_start = .;
        *(.bss .bss.* .sbss .sbss.* COMMON)
        . = ALIGN(4);
        _bss_end = .;
    } > ram
    _stack_top = ORIGIN(ram) + LENGTH(ram);
}
