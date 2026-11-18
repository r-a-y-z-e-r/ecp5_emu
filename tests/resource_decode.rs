use ecp5_emu::{
    database::Database,
    decode::{Catalog, Cram, Report},
};
use std::{fmt::Write, path::PathBuf, process::Command};

fn value<'a>(r: &'a Report, id: &str, name: &str) -> Option<&'a str> {
    r.resources
        .iter()
        .find(|r| r.id == id)
        .unwrap()
        .fields
        .iter()
        .find(|f| f.name == name)
        .unwrap()
        .value
        .as_deref()
}

#[test]
#[ignore = "requires local pytrellis and the pinned LFE5U-85F database"]
fn native_bitstream_and_text_decode_the_same_ebr_dsp_and_init() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let db = Database::discover(None).unwrap();
    let catalogue_path = root.join("docs/inventory/LFE5U-85F/decode_catalog.json");
    let catalogue = Catalog::load(&catalogue_path).unwrap();
    let folder = std::env::temp_dir().join(format!("ecp5-emu-decode-{}", std::process::id()));
    std::fs::create_dir(&folder).unwrap();
    let text_path = folder.join("fixture.config");
    let bit_path = folder.join("fixture.bit");
    let mut config = String::from(".device LFE5U-85F\n");
    for (name, col, types, wid, mode) in [
        ("EBR0", 105, vec!["MIB_EBR0", "MIB_EBR1"], 3_u16, "DP16KD"),
        (
            "EBR3",
            102,
            vec!["MIB_EBR6", "MIB_EBR7", "MIB_EBR8"],
            7_u16,
            "DP16KD",
        ),
        (
            "EBR1",
            107,
            vec!["MIB_EBR2", "MIB_EBR3", "MIB_EBR4"],
            9_u16,
            "PDPW16KD",
        ),
    ] {
        write!(config, ".tile_group").unwrap();
        for (i, kind) in types.iter().enumerate() {
            write!(config, " MIB_R22C{}:{kind}", col + i).unwrap();
        }
        writeln!(
            config,
            "\nenum: {name}.MODE {mode}\nword: {name}.WID {:09b}",
            wid.reverse_bits() >> 7
        )
        .unwrap();
        if mode == "DP16KD" {
            writeln!(
                config,
                "enum: {name}.DP16KD.DATA_WIDTH_A 9\nenum: {name}.DP16KD.DATA_WIDTH_B 2"
            )
            .unwrap();
        } else {
            writeln!(config, "enum: {name}.PDPW16KD.DATA_WIDTH_R 36").unwrap();
        }
    }
    write!(config, ".tile_group").unwrap();
    for i in 0..=4 {
        write!(
            config,
            " MIB_R10C{}:MIB_DSP{i} MIB_R10C{}:MIB2_DSP{i}",
            105 + i,
            105 + i
        )
        .unwrap();
    }
    config.push_str("\nenum: MULT18_0.MODE MULT18X18D\nenum: MULT18_0.REG_INPUTA_CLK CLK1\nenum: MULT18_0.REG_PIPELINE_CLK CLK2\n");
    for wid in [3_u16, 7, 9] {
        writeln!(config, ".bram_init {wid}").unwrap();
        for i in 0..2048 {
            write!(
                config,
                "{:03x}{}",
                (i * 13 + usize::from(wid)) & 511,
                if i % 8 == 7 { '\n' } else { ' ' }
            )
            .unwrap();
        }
    }
    std::fs::write(&text_path, config).unwrap();
    let serializer = r#"
import importlib.util,os,sys
from pathlib import Path
if os.environ.get('ECP5_TRELLIS_LIB'):
    sys.path.insert(0, os.environ['ECP5_TRELLIS_LIB'])
# Prefer the override and normal Python search path, then installs beside PATH.
if importlib.util.find_spec('pytrellis') is None:
    prefixes = [Path(p).resolve().parent for p in os.get_exec_path()]
    candidates = [prefix / lib / 'trellis' for prefix in prefixes for lib in ['lib', 'lib64']]
    # Homebrew is the final fallback, after portable discovery and Unix locations.
    candidates += [Path(p) for p in ['/usr/local/lib/trellis', '/usr/lib/trellis', '/opt/homebrew/lib/trellis']]
    for p in candidates:
        sys.path.append(str(p))
        if importlib.util.find_spec('pytrellis') is not None:
            break
import pytrellis
pytrellis.load_database(sys.argv[1])
chip=pytrellis.ChipConfig.from_string(open(sys.argv[2]).read()).to_chip()
pytrellis.Bitstream.serialise_chip(chip).write_bit(sys.argv[3])
"#;
    let output = Command::new(std::env::var_os("ECP5_PYTHON").unwrap_or_else(|| "python3".into()))
        .args(["-c", serializer])
        .arg(&db)
        .arg(&text_path)
        .arg(&bit_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let selected = vec![
        "LFE5U-85F/R22C105/EBR0".into(),
        "LFE5U-85F/R22C102/EBR3".into(),
        "LFE5U-85F/R22C107/EBR1".into(),
        "LFE5U-85F/R10C105/MULT18_0".into(),
    ];
    let text = Cram::load(&text_path, &db, &catalogue_path).unwrap();
    let bit = Cram::load(&bit_path, &db, &catalogue_path).unwrap();
    let a = catalogue.decode(&text, &selected).unwrap();
    let b = catalogue.decode(&bit, &selected).unwrap();
    assert_eq!(a.cram_fingerprint, b.cram_fingerprint);
    assert_eq!(a.resources, b.resources);
    for id in &selected[..2] {
        assert_eq!(
            value(
                &a,
                id,
                if id.ends_with("EBR0") {
                    "EBR0.MODE"
                } else {
                    "EBR3.MODE"
                }
            ),
            Some("DP16KD")
        );
    }
    assert_eq!(
        value(&a, &selected[0], "EBR0.DP16KD.DATA_WIDTH_A"),
        Some("9")
    );
    assert_eq!(
        value(&a, &selected[0], "EBR0.DP16KD.DATA_WIDTH_B"),
        Some("2")
    );
    assert_eq!(value(&a, &selected[2], "EBR1.MODE"), Some("PDPW16KD"));
    assert_eq!(
        value(&a, &selected[2], "EBR1.PDPW16KD.DATA_WIDTH_R"),
        Some("36")
    );
    assert_eq!(value(&a, &selected[3], "MULT18_0.MODE"), Some("MULT18X18D"));
    assert_eq!(
        value(&a, &selected[3], "MULT18_0.REG_INPUTA_CLK"),
        Some("CLK1")
    );
    assert_eq!(
        value(&a, &selected[3], "MULT18_0.REG_PIPELINE_CLK"),
        Some("CLK2")
    );
    for (id, wid) in selected.iter().zip([3_u16, 7, 9]) {
        let memory = a
            .resources
            .iter()
            .find(|r| &r.id == id)
            .unwrap()
            .memory
            .as_ref()
            .unwrap();
        assert_eq!(memory.status, "bound");
        assert_eq!(memory.wid, Some(wid));
        assert_eq!(memory.image_id, Some(wid));
        assert_eq!(a.images[&wid].words_9bit, b.images[&wid].words_9bit);
        assert_eq!(
            a.images[&wid].words_9bit,
            (0..2048)
                .map(|i| ((i * 13 + usize::from(wid)) & 511) as u16)
                .collect::<Vec<_>>()
        );
    }
    std::fs::remove_dir_all(folder).unwrap();
}
