"""nextpnr --pre-pack hook: reference clock and actual clock-domain rates.

The supplied hb11.lpf constrains pins only. --freq alone would incorrectly
apply 200 MHz to the 25 MHz UART domain as well. Explicit output constraints
also cover tool versions which do not derive PLL/DCS clock constraints.
"""

ctx.addClock("clk_25mhz", 25)
ctx.addClock("fast", 200)
ctx.addClock("c0", 6.25)
ctx.addClock("c1", 4.6875)
ctx.addClock("d0", 6.25)
ctx.addClock("d1", 6.25)
