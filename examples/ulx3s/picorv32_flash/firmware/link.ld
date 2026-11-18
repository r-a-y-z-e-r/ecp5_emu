MEMORY {
    flash (rx) : ORIGIN = 0x1020000c, LENGTH = 64K
    ram (rw) : ORIGIN = 0x1000, LENGTH = 4K
}
SECTIONS {
    .text : { *(.text.start) *(.text .text.*) } > flash
    .rodata : { *(.rodata .rodata.* .srodata .srodata.*) . = ALIGN(4); } > flash
    .bss (NOLOAD) : ALIGN(4) {
        _bss_start = .;
        *(.bss .bss.* .sbss .sbss.* COMMON)
        . = ALIGN(4); _bss_end = .;
    } > ram
    _stack_top = ORIGIN(ram) + LENGTH(ram);
    /DISCARD/ : { *(.comment) *(.riscv.attributes) }
}
