use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
pub struct Folder(pub PathBuf);
impl Folder {
    pub fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "ecp5 flow unit {} {}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("top.v"), "module top; endmodule\n").unwrap();
        fs::write(root.join("top.lpf"), "").unwrap();
        Self(root)
    }
}
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
pub const YOSYS: &str = "2.51. Printing statistics.\n\n=== top ===\n\n        +----------Local Count, excluding submodules.\n        | \n    12788 wires\n    26847 wire bits\n    12788 public wires\n    26847 public wire bits\n        3 ports\n       16 port bits\n    15964 submodules\n        5   CCU2C\n      788   L6MUX21\n    10843   LUT4\n     3553   PFUMX\n      775   TRELLIS_FF\n";
pub const UTIL: &str = "Info: Device utilisation:\nInfo: \t          TRELLIS_IO:      16/    365     4%\nInfo: \t                DCCA:       1/     56     1%\nInfo: \t              DP16KD:       0/    208     0%\nInfo: \t          MULT18X18D:       0/    156     0%\nInfo: \t              ALU54B:       0/     78     0%\nInfo: \t             EHXPLLL:       0/      4     0%\nInfo: \t             IOLOGIC:       0/    224     0%\nInfo: \t          TRELLIS_FF:     775/  83640     0%\nInfo: \t        TRELLIS_COMB:   10859/  83640    12%\nInfo: \t        TRELLIS_RAMW:       0/  10455     0%\n\nInfo: Placed 16 cells based on constraints.\n";
pub const CLOCK: &str = "Info: Max frequency for clock '$glbnet$clk_25mhz$TRELLIS_IO_IN': 448.83 MHz (PASS at 25.00 MHz)\nInfo: Max frequency for clock '$glbnet$clk_25mhz$TRELLIS_IO_IN': 354.99 MHz (PASS at 25.00 MHz)\nInfo: Max delay <async>                                 -> posedge $glbnet$clk_25mhz$TRELLIS_IO_IN: 0.82 ns\nInfo: Program finished normally.\n";
