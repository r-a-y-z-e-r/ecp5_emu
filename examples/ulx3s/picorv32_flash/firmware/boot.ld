MEMORY { rom (rx) : ORIGIN = 0, LENGTH = 4K }
SECTIONS {
    .text : { *(.text.start) *(.text .text.*) } > rom
    .rodata : { *(.rodata .rodata.* .srodata .srodata.*) } > rom
    _stack_top = 8192;
}
