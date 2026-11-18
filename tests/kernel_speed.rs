//! Event-kernel throughput on configurations that ran on the board. Prints
//! reference cycles per second; run with --ignored --nocapture through
//! `rnd run` so the numbers land in the ledger.
mod board;
use board::{compile_board, PERIOD_PS};
use ecp5_emu::{kernel::Time, logic::Logic};
use std::time::Instant;

fn measure(config: &str, cycles: u64) {
    let mut c = compile_board(config, &["L4"]);
    let clock = c.drivers["pin:G2"];
    let start = Instant::now();
    for cycle in 1..=cycles {
        let rise = Time::from_ps((2 * cycle - 1) * PERIOD_PS / 2);
        let fall = Time::from_ps(cycle * PERIOD_PS);
        c.kernel.schedule(rise, clock, Logic::One).unwrap();
        c.kernel.run_until(rise).unwrap();
        c.kernel.schedule(fall, clock, Logic::Zero).unwrap();
        c.kernel.run_until(fall).unwrap();
    }
    let seconds = start.elapsed().as_secs_f64();
    println!(
        "SPEED {config}: {cycles} cycles in {seconds:.2} s = {:.0} cycles/s",
        cycles as f64 / seconds
    );
    let s = c.kernel.statistics();
    let per = |n: u64| n as f64 / cycles as f64;
    println!(
        "  per cycle: signal changes {:.1}, driver changes {:.1}, combinational evaluations {:.1}, register evaluations {:.1}",
        per(s.signal_changes),
        per(s.driver_changes),
        per(s.combinational_evaluations),
        per(s.register_evaluations)
    );
}

#[test]
#[ignore = "benchmark; requires the native image reader and a local Trellis database"]
fn kernel_speed() {
    // KERNEL_SPEED=config:cycles measures one workload, for profiling.
    if let Ok(one) = std::env::var("KERNEL_SPEED") {
        let (config, cycles) = one.split_once(':').unwrap();
        return measure(config, cycles.parse().unwrap());
    }
    measure("hi02_iologic/hi02.config", 50_000);
    measure("hc05_pll_lock/hc05.config", 20_000);
    measure("hi02_iologic/hi02b.config", 10_000);
}
