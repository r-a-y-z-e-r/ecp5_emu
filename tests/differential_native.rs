//! Pseudo-differential LVCMOS33D output (docs/PIO_BANKS.md): the complement
//! pad carries the inverse of the true pad and shares its tristate (sysIO
//! guide FPGA-TN-02032, section 4.11.5). The configuration is the HP02 design
//! (docs/inventory/LFE5U-85F/hp02_lvcmos33d): an ODDRX1F on gp[3] (B9) with
//! IO_TYPE=LVCMOS33D, complement on gn[3] (C10), tristate high for samples
//! t = 1156..1211.
mod board;
use board::{compile_board, pin, PERIOD_PS};
use ecp5_emu::{kernel::Time, logic::Logic};

#[test]
#[ignore = "requires the native image reader and a local Trellis database"]
fn hp02_complement_pad_is_the_inverse_of_the_true_pad() {
    let mut c = compile_board("hp02_lvcmos33d/hp02.config", &["B9", "C10"]);
    let modes = &c.report.pio_modes;
    assert_eq!(
        modes["LFE5U-85F/R0C58/PIOB"].complement_of.as_deref(),
        Some("LFE5U-85F/R0C58/PIOA")
    );
    let reference = c.drivers["pin:G2"];
    let (mut driven, mut released) = (0, 0);
    for cycle in 1..=1300_u64 {
        let rise = Time::from_ps((2 * cycle - 1) * PERIOD_PS / 2);
        let fall = Time::from_ps(cycle * PERIOD_PS);
        c.kernel.schedule(rise, reference, Logic::One).unwrap();
        c.kernel.run_until(rise).unwrap();
        c.kernel.schedule(fall, reference, Logic::Zero).unwrap();
        c.kernel.run_until(fall).unwrap();
        // The value sampled at edge t = cycle on the board.
        let t = cycle;
        let (p, n) = (c.value(&pin("B9")).unwrap(), c.value(&pin("C10")).unwrap());
        if (1156..=1211).contains(&t) {
            assert_eq!((p, n), (Logic::Z, Logic::Z), "released at t = {t}");
            released += 1;
        } else if t > 40 {
            assert!(p.bit().is_some() && n == p.invert(), "t = {t}: {p} {n}");
            driven += 1;
        }
    }
    assert_eq!(released, 56);
    assert!(driven > 1000);
}
