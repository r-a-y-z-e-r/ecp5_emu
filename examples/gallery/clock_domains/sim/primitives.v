`ifdef CDC_SIM
// Stand-ins only for the clock generators. The C++ testbench drives the
// three public variables on independent edge trains. Everything downstream
// is the unchanged RTL, including the inferred dual-clock block RAM.
module EHXPLLL #(
 parameter PLLRST_ENA="DISABLED", INTFB_WAKE="DISABLED", STDBY_ENABLE="DISABLED",
 DPHASE_SOURCE="DISABLED", OUTDIVIDER_MUXA="DIVA", OUTDIVIDER_MUXB="DIVB",
 OUTDIVIDER_MUXC="DIVC", OUTDIVIDER_MUXD="DIVD", CLKI_DIV=5,
 CLKOP_ENABLE="ENABLED", CLKOP_DIV=15, CLKOP_CPHASE=7, CLKOP_FPHASE=0,
 CLKOS_ENABLE="ENABLED", CLKOS_DIV=18, CLKOS_CPHASE=7, CLKOS_FPHASE=0,
 FEEDBK_PATH="CLKOP", CLKFB_DIV=8
)(input RST,STDBY,CLKI,CLKFB,PHASESEL0,PHASESEL1,PHASEDIR,PHASESTEP,
 PHASELOADREG,PLLWAKESYNC,ENCLKOP,ENCLKOS,ENCLKOS2,ENCLKOS3,
 output CLKOP,CLKOS,CLKOS2,CLKOS3,CLKINTFB,LOCK,INTLOCK,REFCLK);
 reg drive40 /* verilator public_flat_rw */ =0;
 reg drive33 /* verilator public_flat_rw */ =0;
 assign CLKOP=drive40;
 assign CLKOS=drive33;
 assign LOCK=1;
 assign CLKOS2=0,CLKOS3=0,CLKINTFB=0,INTLOCK=1,REFCLK=CLKI;
endmodule

module OSCG #(parameter DIV=8)(output OSC);
 reg drive /* verilator public_flat_rw */ =0;
 assign OSC=drive;
endmodule
`endif
