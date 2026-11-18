# Resource configuration decoding

The new `load` path uses the [bound image and the merged catalog of 28 810 fields](CONFIGURATION_STATE.md), preserving the service data of `.bit` through a direct C++ bridge. The `decode` command described below still uses the previous catalog of 14 417 fields and the old bridge; their limits must not be carried over to the new `load`, nor should all modes be considered already executable.

The later collection revealed a separate limit: a match of CRAM/EBR INIT does not mean that CTRL0/CTRL1/SED/USERCODE or the loading process is preserved. Four values are lost in the native text round-trip; the current bridge does not export them. Details and the native probe are in [SYSTEM_STARTUP.md](SYSTEM_STARTUP.md). The new PLL/DCC/DCS and PIO/bank fields and the other research stay as separate catalogs until the next implementation; their scope is summarized in [RESEARCH_STATUS.md](RESEARCH_STATUS.md).

The `decode` command reads `.bit` or `.config`, reconstructs the shared CRAM, and parses the settings of whole resources. The first working catalog covers 14 417 fields: all EBR, MULT18, ALU54, the shared settings of DSP slices, and the previously bound clock and system fields. This is the configuration layer; execution of memory, DSP, and the other hard blocks is still ahead.

## Running and output

From the project root:

```sh
cargo run --release --locked -- decode test.config --output decoded.json
cargo run --release --locked -- decode ~/FPGA/projects/blink/top.bit \
  --resource LFE5U-85F/R22C105/EBR0 --output ebr0.json
```

`--resource ID` can be repeated. Without a filter, the output contains all decoder resources. The filter is applied after the INIT owners are checked across all EBRs. The `--db PATH` and `--catalog PATH` options select the database and the catalog explicitly. An available `pytrellis` and a compatible Python are required; the paths are configured through `ECP5_TRELLIS_LIB` and `ECP5_PYTHON`.

The JSON contains resource identifiers, effective values, statuses, remaining variants, source tiles, defaults, and observed CRAM bits. For EBR, the WID and the status of the link to INIT are added. All INIT images that were read are kept whole, including images without an owner. The CRAM fingerprint is FNV-1a 64, for comparing snapshots, without including the separate INIT memory.

## From the input file to CRAM

`tools/trellis_bridge.py` uses libtrellis to deserialize `.bit` or to apply `.config`. This way the library accounts for defaults, tile groups, and overlapping configuration areas. The bridge passes the whole CRAM to Rust, not only the printed enum settings. The memory images are passed separately through `ChipConfig.bram_data`.

[decode_catalog.json](inventory/LFE5U-85F/decode_catalog.json) is generated from the same 21 987 fragments as the research map. It keeps the requirements on global CRAM bits, the source values, and references to database lines. Before import, the SHA-256 of the database files listed in the catalog are checked; a stale catalog raises an error. This check does not certify the version of the installed binary libtrellis.

The catalog does not yet cover all die settings. A full new decoder for the logic fabric, IO, and the configuration banks needs further work. The source data of these families stay in the general inventory.

The [PLC/LUTRAM collection](PLC_LUTRAM.md) added research patterns for all 91 enums and eight INIT words of PLC2, with a per-bit check. They are not yet connected to this working catalog. An extension will need to keep the shared MODE A/B/C bit, the INIT inversion, the WREMUX/LSR1 and carry/output mux overlaps, and the difference between the physical LUT index and the logical RAM address.

## Fields spanning several tiles

For an enum, the requirements of one variant are merged across all fragments. A variant matches if all its known constraints are met. Among the matched ones, only the patterns that are strict subsets of another matched pattern are dropped. Identical and incomparable sets of constraints stay ambiguous. A larger number of matched bits does not by itself give the right to choose a variant.

| Enum status | Meaning |
|---|---|
| `resolved` | One observable variant remains, described in all fragments |
| `ambiguous` | Several variants remain that cannot be eliminated |
| `partial` | The only candidate is missing from the description of some fragments |
| `unobserved` | The only candidate sets no observable bit |
| `unmatched` | No known variant matches the CRAM |

Defaults are kept as source information. The decoder does not substitute them for an unrecognized result. This matters for DSP, where the defaults of individual fragments differ.

The [DQS/DDRDLL analysis](DQS_CONTROL.md) additionally lists the partial MODE masks and the FORCE_MAX_DELAY combinations. A match of a record pattern, including an empty NONE, does not by itself certify that the combination is valid in hardware or that the internal paths are active. Checking the resource context remains a separate task.

For words, the observations of each logical position are merged. An empty group `-` sets no constant. A missing observation gives `partial`; contradictory fragments or diverging copies of one bit give `conflict`. A fully known word has `resolved` and a binary value. The `word_bits_lsb0` array keeps unknown positions separately. For the EBR fields of the DP16KD/PDPW16KD modes, the applicability to the reconstructed MODE is additionally stated.

## Linking EBR to INIT

Each EBR has a nine-bit WID. The decoder assembles it from all tiles, including the 5+4 split at EBR3, and reverses the order of the nine bits by the `nextpnr::write_bram` rule. The result is matched against the ID of `.bram_init`.

A full image has 2 048 nine-bit words. The text parser now rejects values greater than `0x1ff`; three printed hex digits do not mean 12 bits of memory. The data are for now kept in the original libtrellis format, without conversion to port addresses of the selected width.

The mapping of 1/2/4/9/18/36-wide ports to this image and the INITVAL conversion are collected in [EBR_ADDRESSING.md](EBR_ADDRESSING.md). The same document describes the nine-bit masks on the low address pins and the CSDECODE reversal, taking into account the already encoded input inversion. These rules are researched but not yet included in memory execution.

[EBR_CONTROL.md](EBR_CONTROL.md) shows that synthesis can move resets and output pipelines into the external fabric. The decoder and the subsequent model must rely on the actual settings of the EBR and the surrounding resources. The table of hardware control priorities has not been measured yet; B01–B08 are included in the future board experiments.

| Link status | Meaning |
|---|---|
| `bound` | An active EBR, the only known owner of the WID, an image of full length |
| `inactive` | MODE NONE is reconstructed |
| `indeterminate` | MODE or WID is not established |
| `duplicate_wid` | Several potentially active EBRs claim the same WID |
| `missing_init` | No image with such an ID |
| `invalid_image_length` | The image exists, but its length differs from 2 048 words |

A missing or short INIT is not padded with zeros. For text, the `explicit_text_words` mark is kept. For `.bit`, `libtrellis_decoded_without_write_mask` is used: the library does not return the mask of the words that were actually written and may fill the unsent parts with zeros. Therefore `complete_length` certifies the length of the resulting array, not the completeness of the memory write by the physical stream.

Dynamic reconfiguration, partial streams, and the whole range of valid hardware IDs have not been checked yet. The boundary between the ten-bit address of the memory command and the nine-bit WID remains a separate question. The `bound` status confirms the match of the data in the current snapshot; it does not certify the hardware behavior of the memory.

## Verification

Eighteen Rust tests and fifteen portable Python checks pass. The decoder checks include composite MODE, different defaults, incomplete descriptions, incomparable enum patterns, empty word groups, contradictory copies of bits, all 512 WID values, and ambiguous INIT owners.

The local integration test creates a configuration with three EBRs: DP16KD, PDPW16KD, and EBR3 with a split WID. It sets different port widths, three full INIT images, and the clock settings of the MULT18 registers, then serializes the configuration with the Trellis library. When reading `.config` and the resulting `.bit`, the whole CRAM, the fields of the checked resources, and all INIT words matched. This checks the conversion path on a constructed example, without hardware comparison and without an exhaustive sweep of modes.

```sh
cargo test --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
python3 -m unittest discover -s tests -p 'test_*.py' -v
cargo test --release --locked --test resource_decode -- --ignored
```

Two earlier integration checks of the prototype also passed: a counter over 65 538 cycles and the match of running the existing blink from `.config` and `.bit` over 256 cycles. They need the local database and `ECP5_TEST_BIT` pointing to blink/top.bit.

The next step for EBR is to clarify the control transitions and the access uncertainty per questions B01–B08, then to define the executable state and the read/write events on the collected addressing. For DSP, the owners of the shared registers must be established. The decoder provides the configuration and the bit provenance for this work. The full graph structure and the common event scheduler remain open stages of the plan.
