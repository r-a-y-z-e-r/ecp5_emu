"""nextpnr --pre-route hook: retain X13's direct G2 -> bank 7 ECLK1 path.
The extra support loads must not let the router choose bank 6's neighbor ECLK.
No configuration enum is patched; nextpnr still writes the ECLKSYNC MODE.
"""
# nextpnr supplies ctx and STRENGTH_USER in the hook's globals.
prefix = [
    'X2/Y46/-2_1_JPADDIA_PIO->-2_2_JPADDI',
    'X2/Y46/-2_2_JPADDI->-2_2_JINCK',
    'X2/Y46/-2_2_JINCK->-2_-46_G_JPCLKT61',
    'X2/Y46/-2_-46_G_JPCLKT61->-2_0_JECLKI1',
    'X2/Y46/-2_0_JECLKI1->-2_0_ECLKI_ECLKSYNC1',
]
internal = 'X2/Y46/-2_0_ECLKI_ECLKSYNC1->-2_0_JECLKO_ECLKSYNC1'
suffix = [
    'X2/Y46/-2_0_JECLKO_ECLKSYNC1->-2_0_SYNCECLK1',
    'X2/Y46/-2_0_SYNCECLK1->-2_0_JECLK1',
    'X2/Y46/-2_0_JECLK1->-2_-46_G_BANK7ECLK1',
    'X2/Y46/-2_-46_G_BANK7ECLK1->-2_0_CLKI1',
    'X2/Y46/-2_0_CLKI1->-2_0_CLKI_CLKDIV1',
]
control = any(name == 'sync' for name, _ in ctx.cells)
raw = ctx.cells['sync' if control else 'u'].ports['ECLKI' if control else 'CLKI'].net
output = ctx.cells['sync'].ports['ECLKO'].net if control else raw
assert raw.name == ctx.cells['clk$tr_io'].ports['O'].net.name, 'divider must use raw G2'
for pips, net in [(prefix, raw), ([] if control else [internal], raw), (suffix, output)]:
    for pip in pips:
        source = ctx.getPipSrcWire(pip)
        bound = ctx.getBoundWireNet(source)
        if bound is None:
            ctx.bindWire(source, net, STRENGTH_USER)
        else:
            assert bound.name == net.name, (pip, bound.name, net.name)
        bound = ctx.getBoundPipNet(pip)
        if bound is None:
            ctx.bindPip(pip, net, STRENGTH_USER)
        else:
            assert bound.name == net.name, (pip, bound.name, net.name)
