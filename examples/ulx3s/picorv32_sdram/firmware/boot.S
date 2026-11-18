# EBR at zero holds this ROM and the firmware image at offset 0x100.
# The payload is linked at 0x40000000, and all execution after this copy
# (including the stack and .bss) goes through the SDRAM bus.
    .section .text.start
    .global _start
_start:
    li t0, 0x100
    li t1, 0x40000000
    li t2, 0x2000
1:  lw t3, 0(t0)
    sw t3, 0(t1)
    addi t0, t0, 4
    addi t1, t1, 4
    bltu t0, t2, 1b
    li t0, 0x40000000
    jr t0
