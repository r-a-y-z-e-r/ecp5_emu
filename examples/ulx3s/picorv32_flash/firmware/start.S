# Entry point at address 0: clear .bss, set the stack at the top of RAM, run main.
    .section .text.start
    .global _start
_start:
    la t0, _bss_start
    la t1, _bss_end
1:  bgeu t0, t1, 2f
    sw zero, 0(t0)
    addi t0, t0, 4
    j 1b
2:  la sp, _stack_top
    call main
3:  j 3b
