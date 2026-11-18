#![allow(dead_code)]
pub mod reference;
// Shared synthetic architecture fixture for integration tests.
use ecp5_emu::model::DeviceModel;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);

/// Optional generated-C snapshots for comparing DSP emission across revisions.
pub fn snapshot_dsp_source(label: &str, kernel: &ecp5_emu::kernel::Kernel) {
    if let Some(dir) = std::env::var_os("DSP_SOURCE_SNAPSHOT") {
        write(
            &PathBuf::from(dir).join(format!("{label}.c")),
            kernel.fast_program().unwrap().source,
        );
    }
}

pub struct Fixture {
    pub root: PathBuf,
    pub db: PathBuf,
    pub evidence: PathBuf,
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

impl Fixture {
    pub fn with_system() -> Self {
        let f = Self::with_lutram();
        let path = f.db.join("ECP5/LFE5U-85F/tilegrid.json");
        let mut grid: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        for (name, kind, bit) in [
            ("SYS0_R1C1:EFB0_PICB0", "EFB0_PICB0", 16),
            ("SYS2_R1C1:EFB2_PICB0", "EFB2_PICB0", 24),
        ] {
            grid[name] =
                json!({"type":kind,"start_frame":0,"start_bit":bit,"cols":2,"rows":8,"sites":[]});
        }
        write(&path, grid.to_string());
        write(
            &f.db.join("ECP5/tiledata/EFB0_PICB0/bits.db"),
            r#"
.config_enum GSR.GSRMODE
NONE -
ACTIVE_HIGH F0B0
ACTIVE_LOW F0B0 F0B1
.config_enum GSR.SYNCMODE
NONE -
ASYNC -
SYNC F0B2
.config_enum SYSCONFIG.WAKE_UP
4 !F0B3 !F0B4
21 F0B3 F0B4
.config_enum SYSCONFIG.DONEPHASE
T0 !F0B3 !F0B4
T1 F0B3 !F0B4
T2 !F0B3 F0B4
T3 F0B3 F0B4
.config_enum SYSCONFIG.DONE_EX
OFF -
ON F0B5
.config_enum SYSCONFIG.GOEPHASE
T1 -
T2 F0B6
.config_enum SYSCONFIG.GSRPHASE
T2 !F1B0
T1 F1B0
.config_enum SYSCONFIG.GWEPHASE
T2 !F1B1
T3 F1B1
"#,
        );
        write(
            &f.db.join("ECP5/tiledata/EFB2_PICB0/bits.db"),
            ".config_enum SYSCONFIG.DONE_EX\nOFF -\nON F0B0\n",
        );
        let path = f.evidence.join("resource_instances.csv");
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str(
            "LFE5U-85F/R1C1/GSR,GSR,1,1,GSR,SYS0_R1C1:EFB0_PICB0,,True,GSR,synthetic_system\n",
        );
        write(&path, text);
        let path = f.evidence.join("resource_ports.csv");
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str(
            "LFE5U-85F/R1C1/GSR,GSR,in,1,1,JGSR_GSR\nLFE5U-85F/R1C1/GSR,CLK,in,1,1,JCLK_GSR\n",
        );
        write(&path, text);
        let path = f.db.join("ECP5/tiledata/PLC2/bits.db");
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace(
                ".config_enum SLICEA.GSR\nENABLED -",
                ".config_enum SLICEA.GSR\nENABLED -\nDISABLED F9B6",
            )
            .replace(
                ".config_enum SLICEA.REG0.REGSET\nRESET -",
                ".config_enum SLICEA.REG0.REGSET\nRESET -\nSET F9B7",
            );
        write(&path, text);
        let path = f.evidence.join("device_inventory.json");
        let mut inv: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        for kind in ["EFB0_PICB0", "EFB2_PICB0"] {
            let path = format!("ECP5/tiledata/{kind}/bits.db");
            inv["source_files"]
                .as_array_mut()
                .unwrap()
                .push(json!({"sha256":hash(&fs::read(f.db.join(&path)).unwrap()),"path":path}));
        }
        write(&path, inv.to_string());
        f.refresh_database();
        f
    }
    pub fn with_lutram() -> Self {
        let f = Self::new();
        let path = f.db.join("ECP5/LFE5U-85F/tilegrid.json");
        let mut grid: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        grid["R1C1:PLC2"]["cols"] = json!(10);
        grid["R1C1:PLC2"]["rows"] = json!(16);
        let mut instances = fs::read_to_string(f.evidence.join("resource_instances.csv")).unwrap();
        let mut bindings = fs::read_to_string(f.evidence.join("site_bindings.csv")).unwrap();
        for (index, slice) in ['B', 'C', 'D'].into_iter().enumerate() {
            grid["R1C1:PLC2"]["sites"].as_array_mut().unwrap().push(json!({"name":format!("SLICE{slice}"),"pos_row":"-2","pos_col":(21+index).to_string()}));
            instances.push_str(&format!("LFE5U-85F/R1C1/SLICE_{slice},SLICE,1,1,SLICE_{slice},R1C1:PLC2,LFE5U-85F/R1C1/PLC2,True,SLICE{slice},logic\n"));
            bindings.push_str(&format!("R1C1:PLC2,{},SLICE{slice},SLICE,-2,{},LFE5U-85F/R1C1/SLICE_{slice},instance,logic\n",index+1,index+21));
        }
        write(&path, grid.to_string());
        write(&f.evidence.join("resource_instances.csv"), instances);
        write(&f.evidence.join("site_bindings.csv"), bindings);
        write(
            &f.evidence.join("plc_research/node_templates.csv"),
            include_str!("../../docs/inventory/LFE5U-85F/plc_research/node_templates.csv"),
        );
        write(
            &f.evidence.join("plc_research/native_ports.csv"),
            include_str!("../../docs/inventory/LFE5U-85F/plc_research/native_ports.csv"),
        );
        let mut bits = String::new();
        for (z, slice) in ['A', 'B', 'C', 'D'].into_iter().enumerate() {
            for k in 0..2 {
                bits.push_str(&format!(
                    "\n.config SLICE{slice}.K{k}.INIT 0000000000000000\n"
                ));
                for bit in 0..16 {
                    bits.push_str(&format!("!F{}B{bit}\n", 2 * z + k));
                }
                for port in ["A", "B", "C", "D", "M"] {
                    bits.push_str(&format!(
                        "\n.config_enum SLICE{slice}.{port}{k}MUX\n{port}{k} -\n1 F9B15\n"
                    ));
                }
                for (field, choices) in [
                    ("LSRMODE", "LSR -"),
                    ("REGSET", "RESET -"),
                    ("SD", if slice == 'C' { "0 -\n1 F9B4" } else { "0 -" }),
                ] {
                    bits.push_str(&format!(
                        "\n.config_enum SLICE{slice}.REG{k}.{field}\n{choices}\n"
                    ));
                }
                bits.push_str(&format!(
                    "\n.fixed_conn Q{} Q{}_SLICE\n",
                    2 * z + k,
                    2 * z + k
                ));
            }
            bits.push_str(&format!(
                "\n.fixed_conn CLK{z}_SLICE MUXCLK{z}\n.fixed_conn LSR{z}_SLICE MUXLSR{z}\n"
            ));
            bits.push_str(&format!(
                "\n.config_enum SLICE{slice}.MODE\nLOGIC -\nCCU2 F8B{}\n",
                z + 1
            ));
            if slice != 'D' {
                bits.push_str(&format!(
                    "{} F8B0\n",
                    if slice == 'C' { "RAMW" } else { "DPRAM" }
                ));
            }
            bits.push_str(&format!("\n.config_enum SLICE{slice}.CEMUX\n1 -\n\n.config_enum SLICE{slice}.GSR\nENABLED -\n\n.mux MUXCLK{z}\nCLK0 -\n\n.mux MUXLSR{z}\nLSR0 -\n"));
        }
        for clock in 0..2 {
            bits.push_str(&format!("\n.config_enum CLK{clock}.CLKMUX\nCLK !F9B0\nINV F9B0\n\n.config_enum LSR{clock}.LSRMUX\nLSR !F9B1\nINV F9B1\n\n.config_enum LSR{clock}.SRMODE\nASYNC -\n"));
        }
        bits.push_str(
            "\n.config_enum SLICEA.WREMUX\nWRE -\nINV F9B1\n1 F9B2 F9B3\n0 F9B1 F9B2 F9B3\n",
        );
        for bit in 0..4 {
            for slice in ['A', 'B'] {
                bits.push_str(&format!(
                    "\n.fixed_conn WAD{bit}{slice}_SLICE WADO{bit}C_SLICE\n"
                ));
            }
        }
        for (lane, slice) in ['A', 'B'].into_iter().enumerate() {
            bits.push_str(&format!(
                "\n.fixed_conn WCK{lane}_SLICE CLK1\n.fixed_conn WRE{lane}_SLICE LSR1\n"
            ));
            for k in 0..2 {
                bits.push_str(&format!(
                    "\n.fixed_conn WD{k}{slice}_SLICE WDO{}C_SLICE\n",
                    2 * lane + k
                ));
            }
        }
        write(&f.db.join("ECP5/tiledata/PLC2/bits.db"), bits);
        let path = f.evidence.join("device_inventory.json");
        let mut inv: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        inv["device_info"]["frames"] = json!(12);
        inv["device_info"]["bits_per_frame"] = json!(32);
        write(&path, inv.to_string());
        f.refresh_database();
        f
    }
    pub fn with_compiler() -> Self {
        let f = Self::with_kernel();
        let path = f.db.join("ECP5/tiledata/PLC2/bits.db");
        let mut data = fs::read_to_string(&path).unwrap().replace(
            ".mux F0\nA0 F1B1 !F0B0\nG_HPBX0000 -",
            ".mux F0\nF0_SLICE -\nF5A_SLICE F1B1",
        );
        data.push_str("\n.fixed_conn DI0_SLICE F0\n.fixed_conn CLK0_SLICE MUXCLK0\n.fixed_conn LSR0_SLICE MUXLSR0\n\n.mux MUXCLK0\nCLK0 -\n\n.mux MUXLSR0\nLSR0 -\n");
        for (name, choices) in [
            ("SLICEA.MODE", "LOGIC -\nCCU2 F0B0\nDPRAM F0B1"),
            ("SLICEA.REG0.LSRMODE", "LSR -\nPRLD F0B6"),
            ("SLICEA.REG1.LSRMODE", "LSR -\nPRLD F0B6"),
            ("SLICEA.REG0.REGSET", "RESET -\nSET F0B2"),
            ("SLICEA.REG1.REGSET", "RESET -\nSET F0B2"),
            ("SLICEA.REG0.SD", "1 -\n0 F0B3"),
            ("SLICEA.REG1.SD", "1 -\n0 F0B3"),
            ("SLICEA.CEMUX", "1 -\nCE F1B1\nINV F1B2\n0 F1B1 F1B2"),
            ("SLICEA.GSR", "ENABLED -"),
            ("SLICEA.CCU2.INJECT1_0", "YES -\nNO F0B5"),
            ("CLK0.CLKMUX", "CLK -\nINV F1B3"),
            ("LSR0.LSRMUX", "LSR -\nINV F1B5"),
            ("LSR0.SRMODE", "ASYNC -\nLSR_OVER_CE F1B4"),
            ("SLICEA.M0MUX", "M0 -\n1 F1B0"),
            ("SLICEA.M1MUX", "M1 -\n1 F1B0"),
        ] {
            data.push_str(&format!("\n.config_enum {name}\n{choices}\n"));
        }
        for p in ["A", "B", "C", "D"] {
            data.push_str(&format!("\n.config_enum SLICEA.{p}0MUX\n{p}0 -\n1 F1B6\n"));
        }
        write(&path, data);
        let path = f.evidence.join("plc_research/native_ports.csv");
        let mut data = fs::read_to_string(&path).unwrap();
        for (bel, kind, z, ports) in [
            (
                "SLICEA.K0",
                "TRELLIS_COMB",
                0,
                vec![
                    ("FCI", "in", "FCI_SLICE"),
                    ("FCO", "out", "FCOA_SLICE"),
                    ("F1", "in", "F1_SLICE"),
                    ("OFX", "out", "F5A_SLICE"),
                    ("M", "in", "M0_SLICE"),
                ],
            ),
            (
                "SLICEA.FF0",
                "TRELLIS_FF",
                1,
                vec![
                    ("DI", "in", "DI0_SLICE"),
                    ("CLK", "in", "CLK0_SLICE"),
                    ("LSR", "in", "LSR0_SLICE"),
                    ("CE", "in", "CE0_SLICE"),
                    ("M", "in", "M0_SLICE"),
                ],
            ),
            (
                "SLICEA.FF1",
                "TRELLIS_FF",
                5,
                vec![
                    ("DI", "in", "DI1_SLICE"),
                    ("CLK", "in", "CLK0_SLICE"),
                    ("LSR", "in", "LSR0_SLICE"),
                    ("CE", "in", "CE0_SLICE"),
                    ("M", "in", "M1_SLICE"),
                ],
            ),
        ] {
            for (port, direction, wire) in ports {
                data.push_str(&format!(
                    "split,{bel},{kind},{z},{port},{direction},0,0,{wire}\n"
                ));
            }
        }
        write(&path, data);
        let path = f.evidence.join("iologic_research/resource_ports.csv");
        let mut data = fs::read_to_string(&path).unwrap();
        data.push_str(
            "LFE5U-85F/R1C1/PIOA,I,in,1,1,PADDOA_PIO\nLFE5U-85F/R1C1/PIOA,T,in,1,1,PADDTA_PIO\n",
        );
        write(&path, data);
        f.refresh_database();
        f
    }
    pub fn with_kernel() -> Self {
        let f = Self::with_storage();
        let path = f.evidence.join("plc_research/node_templates.csv");
        let mut data = fs::read_to_string(&path).unwrap();
        data.push_str("split,SLICEA.FF1,TRELLIS_FF,5,split_view\n");
        write(&path, data);
        let path = f.evidence.join("plc_research/native_ports.csv");
        let mut data = fs::read_to_string(&path).unwrap();
        data.push_str("split,SLICEA.FF1,TRELLIS_FF,5,Q,out,0,0,Q1_SLICE\n");
        for name in ["A", "B", "C", "D"] {
            data.push_str(&format!(
                "split,SLICEA.K0,TRELLIS_COMB,0,{name},in,0,0,{name}0_SLICE\n"
            ));
        }
        write(&path, data);
        f.seal();
        f
    }
    pub fn with_storage() -> Self {
        let f = Self::new();
        let path = f.db.join("ECP5/tiledata/PLC2/bits.db");
        let mut data = fs::read_to_string(&path).unwrap();
        data.push_str("\n.config SLICEA.K0.INIT 1111111111111111\n");
        for frame in 0..2 {
            for bit in 0..8 {
                data.push_str(&format!("!F{frame}B{bit}\n"));
            }
        }
        write(&path, data);
        for (name, line) in [
            (
                "plc_research/node_templates.csv",
                "split,SLICEA.FF0,TRELLIS_FF,1,split_view\n",
            ),
            (
                "plc_research/native_ports.csv",
                "split,SLICEA.FF0,TRELLIS_FF,1,Q,out,0,0,Q0_SLICE\n",
            ),
        ] {
            let path = f.evidence.join(name);
            let mut data = fs::read_to_string(&path).unwrap();
            data.push_str(line);
            write(&path, data);
        }
        f.refresh_database();
        f
    }
    pub fn refresh_database(&self) {
        let path = self.evidence.join("device_inventory.json");
        let mut inv: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        for source in inv["source_files"].as_array_mut().unwrap() {
            source["sha256"] = json!(hash(
                &fs::read(self.db.join(source["path"].as_str().unwrap())).unwrap()
            ));
        }
        write(&path, inv.to_string());
        self.seal();
    }
    pub fn add_ebr(&self, name: &str) {
        let path = self.evidence.join("resource_instances.csv");
        let mut data = fs::read_to_string(&path).unwrap();
        data.push_str(&format!(
            "LFE5U-85F/R1C1/{name},EBR,1,1,{name},R1C1:PLC2,,True,{name},synthetic_test\n"
        ));
        write(&path, data);
        self.seal();
    }
    pub fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "ecp5-model-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let fixture = Self {
            db: root.join("database"),
            evidence: root.join("evidence"),
            root,
        };
        let grid = json!({
            "R1C1:PLC2":{"type":"PLC2","start_frame":2,"start_bit":3,"cols":2,"rows":8,
                "sites":[{"name":"SLICEA","pos_row":"-2","pos_col":"20"}]},
            "CIB_R1C1:CIB":{"type":"CIB","start_frame":0,"start_bit":4,"cols":2,"rows":8,
                "sites":[{"name":"CIBTEST","pos_row":"30","pos_col":"20"}]}
        });
        write(
            &fixture.db.join("ECP5/LFE5U-85F/tilegrid.json"),
            grid.to_string(),
        );
        write(
            &fixture.db.join("ECP5/LFE5U-85F/iodb.json"),
            json!({"packages":{"CABGA381":{"G1":{"row":1,"col":1,"pio":"A"}}}}).to_string(),
        );
        write(
            &fixture.db.join("ECP5/LFE5U-85F/globals.json"),
            json!({
                "quadrants":{"UL":{"x0":0,"y0":0,"x1":1,"y1":1}},
                "taps":{"C1":{"lx0":0,"lx1":0,"rx0":1,"rx1":1}},
                "spines":{"UL1":{"x":0,"y":1}}
            })
            .to_string(),
        );
        write(&fixture.db.join("ECP5/tiledata/PLC2/bits.db"),
            ".mux F0\nA0 F1B1 !F0B0\nG_HPBX0000 -\n\n.fixed_conn A0_SLICE A0\n.fixed_conn ECLKO ECLKI\n");
        write(
            &fixture.db.join("ECP5/tiledata/CIB/bits.db"),
            ".fixed_conn OUT W1_IN\n\n.mux DST\n25K_FOO F0B0\nN2_BAR !F0B0\n",
        );
        let source_paths = [
            "ECP5/LFE5U-85F/tilegrid.json",
            "ECP5/LFE5U-85F/iodb.json",
            "ECP5/LFE5U-85F/globals.json",
            "ECP5/tiledata/PLC2/bits.db",
            "ECP5/tiledata/CIB/bits.db",
        ];
        let sources: Vec<_> = source_paths
            .iter()
            .map(|p| json!({"path":p,"sha256":hash(&fs::read(fixture.db.join(p)).unwrap())}))
            .collect();
        write(
            &fixture.evidence.join("device_inventory.json"),
            json!({"device":"LFE5U-85F","database_revision":"db-test",
            "trellis_revision":"trellis-test","source_files":sources,
            "device_info":{"frames":4,"bits_per_frame":16,"max_row":1,"max_col":1}})
            .to_string(),
        );
        write(&fixture.evidence.join("resource_instances.csv"),
            "resource_id,family,fabric_row,fabric_col,name,anchor_tile,parent_id,native_bel,native_name,rule\n\
LFE5U-85F/R1C1/PLC2,PLC2,1,1,PLC2,R1C1:PLC2,,False,,logic\n\
LFE5U-85F/R1C1/SLICE_A,SLICE,1,1,SLICE_A,R1C1:PLC2,LFE5U-85F/R1C1/PLC2,True,SLICEA,logic\n\
LFE5U-85F/R1C1/PIOA,PIO,1,1,PIOA,CIB_R1C1:CIB,,True,PIOA,io_anchor\n");
        write(&fixture.evidence.join("site_bindings.csv"),
            "tile,site_index,site_name,label_family,vendor_pos_row,vendor_pos_col,resource_id,relation,rule\n\
R1C1:PLC2,0,SLICEA,SLICE,-2,20,LFE5U-85F/R1C1/SLICE_A,instance,logic\n\
CIB_R1C1:CIB,0,CIBTEST,CIBTEST,30,20,,unresolved,unresolved\n");
        write(
            &fixture.evidence.join("resource_ports.csv"),
            "resource_id,port,direction,wire_row,wire_col,wire\n",
        );
        write(&fixture.evidence.join("iologic_research/resource_ports.csv"),
            "resource_id,port,direction,wire_row,wire_col,wire\nLFE5U-85F/R1C1/PIOA,O,out,1,1,JPADDIA_PIO\n");
        write(&fixture.evidence.join("plc_research/node_templates.csv"),
            "view,native_name,native_type,z,interpretation\ncoarse,SLICEA,SLICE,0,coarse_view\nsplit,SLICEA.K0,TRELLIS_COMB,0,split_view\n");
        write(
            &fixture.evidence.join("plc_research/native_ports.csv"),
            "view,bel,bel_type,z,port,direction,wire_row_offset,wire_col_offset,wire\n\
coarse,SLICEA,SLICE,0,A0,in,0,0,A0_SLICE\nsplit,SLICEA.K0,TRELLIS_COMB,0,F,out,0,0,F0_SLICE\n",
        );
        write(
            &fixture
                .evidence
                .join("package_system_research/physical_pins.csv"),
            "package,ball,BSDL_function,PAD,pinout_function,dual_function,bank,kind,evidence\n\
CABGA381,G1,PT1A,1,PT1A,,0,GPIO,native_IO_verified\n\
CABGA381,T1,TCK,2,TCK,,8,dedicated_power_ground_or_NC,public_package_metadata_only\n",
        );
        fixture.seal();
        fixture
    }
    pub fn seal(&self) {
        let names = [
            "device_inventory.json",
            "resource_instances.csv",
            "site_bindings.csv",
            "resource_ports.csv",
            "iologic_research/resource_ports.csv",
            "plc_research/node_templates.csv",
            "plc_research/native_ports.csv",
            "package_system_research/physical_pins.csv",
        ];
        let files: Vec<_> = names
            .iter()
            .map(|p| json!({"path":p,"sha256":hash(&fs::read(self.evidence.join(p)).unwrap())}))
            .collect();
        write(&self.evidence.join("model_manifest.json"),json!({"schema_version":1,"identity_schema_version":1,
            "device":"LFE5U-85F","database_revision":"db-test","trellis_revision":"trellis-test","files":files}).to_string());
    }
    pub fn load(&self) -> DeviceModel {
        DeviceModel::load(&self.db, &self.evidence).unwrap()
    }
    pub fn replace(&self, name: &str, old: &str, new: &str) {
        let p = self.evidence.join(name);
        let data = fs::read_to_string(&p).unwrap();
        assert!(data.contains(old));
        write(&p, data.replace(old, new));
        self.seal();
    }
    pub fn error(&self) -> String {
        DeviceModel::load(&self.db, &self.evidence)
            .err()
            .expect("expected invalid fixture")
            .to_string()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
