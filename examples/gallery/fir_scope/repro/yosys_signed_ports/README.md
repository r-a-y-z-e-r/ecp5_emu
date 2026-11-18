# yosys signed-port re-elaboration failure

`run.sh` synthesises this tiny design twice with yosys 0.70
(`read_verilog ...; synth_ecp5 -top top`). The two runs differ only in
the `dsp_mac` port list: `dsp_mac_signed.v` declares
`input signed [17:0] a, b, output signed [53:0] r`, `dsp_mac_unsigned.v`
drops the `signed` keyword. Everything else is identical.

Observed with yosys 0.70 (git sha1 0c11ea098be065e585a90a06e7b7da49936cf231):

```
ERROR: Module `\gallery_io' referenced in module `\top' in cell `\io' is not part of the design.
```

for the signed variant; the unsigned variant passes. (Which sibling the
message names — `filters` in the original demo, `gallery_io` here —
depends on re-elaboration order; the mechanism is the same.) The log shows
"Reprocessing module" events for the signed-port modules: yosys keeps
the AST of a signed-port module and re-elaborates it at the final
hierarchy check, after `flatten` inside `synth_ecp5` has deleted the
sibling modules (`filters`, `gallery_io`). The re-elaboration of `top`
then references modules that no longer exist.

Workaround used by the demo: no `signed` on any module port; sign is
declared on the wires inside the modules and at the use sites instead
(see the comment in `../../generate.py`). This is functionally free for
the scalar DSP pins.
