/* Everything lives in the 8 KB block RAM at 0; the stack grows down from its top. */
MEMORY { ram (rwx) : ORIGIN = 0, LENGTH = 8K }
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
