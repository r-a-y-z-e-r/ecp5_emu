# EBR, DSP, and the origin of their configuration

This is the initial inventory. [EBR execution](EBR_EXECUTION.md) and the [first MULT18 profile](DSP_EXECUTION.md) were added later; the boundaries of the initial research pass are kept below.

The map is extended to all 208 EBRs, 156 MULT18, and 78 ALU54 of the target LFE5U-85F. For each block, the interface and the related configuration fragments are stored. 78 structural groups of DSP slices inside 39 DSP blocks are added. The first settings decoder and the binding of INIT to the EBR work; the computational behavior of the memory and of the DSP is not yet implemented. The reading rules and the run commands are given in the [decoder description](CONFIG_DECODER.md).

## Interfaces and hierarchy

| Family | Instances | Ports per instance | Total ports |
|---|---:|---:|---:|
| EBR / DP16KD | 208 | 116 | 24 128 |
| MULT18 / MULT18X18D | 156 | 233 | 36 348 |
| ALU54 / ALU54B | 78 | 449 | 35 022 |

All 95 498 new ports matched the constructors of the installed libtrellis exactly: BEL and port names, directions, coordinates, and wire names. The combined check with the previously added clock and system objects covers 502 calls and 96 282 ports. The full current export contains 96 474 ports, including another 192 DCC/DCS ports that were not part of this check. This is a check of the library interfaces, not of the hardware behavior or of the whole routing.

In a DSP group, the left and right slices are singled out. For each, the parent group contains two MULT18 and one ALU54. The existing IDs of these primitives are kept; their parents are refined. DSP_SLICE is a structural grouping with `native_bel=False`, not an additional computational primitive and not a new storage.

## Bit map and summaries

| Set | Configuration field records |
|---|---:|
| EBR | 7 384 |
| MULT18 | 9 087 |
| ALU54 | 5 187 |
| Shared settings of the left/right DSP slice | 78 |
| Previously described clock and system resources | 251 |
| Total | 21 987 |

These are fragment records per instance, not a count of independent switches. They are grouped into 14 417 logical fields of the form "resource + name + kind". 65 634 references to 54 297 unique CRAM bits are stored. Bits mentioned by several fields stay with all owners.

| File | Purpose |
|---|---|
| [decode_catalog.json](inventory/LFE5U-85F/decode_catalog.json) | Composite fields and global-bit requirements for the working Rust decoder |
| [config_field_bindings.csv](inventory/LFE5U-85F/config_field_bindings.csv) | Each source field record, its resource, tile, default, and source |
| [config_field_bits.csv](inventory/LFE5U-85F/config_field_bits.csv) | Local and CRAM coordinates |
| [config_field_groups.csv](inventory/LFE5U-85F/config_field_groups.csv) | Composition of the whole field, all defaults, and identical full enum write patterns |
| [enum_fragment_aliases.csv](inventory/LFE5U-85F/enum_fragment_aliases.csv) | 7 015 classes of identical enum patterns within individual fragments |
| [cross_resource_config_bits.csv](inventory/LFE5U-85F/cross_resource_config_bits.csv) | 6 475 bits that belong to several architectural resources |
| [config_word_layout.csv](inventory/LFE5U-85F/config_word_layout.csv) | 36 668 records of the bit-by-bit word layout, including the empty places of fragments |
| [config_option_conflicts.csv](inventory/LFE5U-85F/config_option_conflicts.csv) | Check of requirements of 0 and 1 on one bit in one composite enum variant; there are no such cases now |

Equality of full patterns is checked only for variants known to all fragments of the field. Different write patterns do not prove that reading is unambiguous: the working decoder keeps the ambiguity of identical and incomparable patterns, and accounts for empty groups and incomplete descriptions. Polarities and source variants are stored in full in device_inventory.json.

## EBR: two or three tiles per block

The split is confirmed by the jobs `041-ebr_config`, `047-copy_ebr_config`, the Trellis architecture description, and `nextpnr::get_bram_tiles`:

| Block | Ordinary configuration types |
|---|---|
| EBR0 | MIB_EBR0, MIB_EBR1 |
| EBR1 | MIB_EBR2, MIB_EBR3, MIB_EBR4 |
| EBR2 | MIB_EBR4, MIB_EBR5, MIB_EBR6 |
| EBR3 | MIB_EBR6, MIB_EBR7, MIB_EBR8 |

SPINE and CMUX replace the corresponding fragments, keeping the EBR settings. A single MIB_EBR4 or MIB_EBR6 contains the fields of two neighboring blocks, so all the settings of the tile cannot be assigned to one resource. All 208 blocks were checked across the full range of their two or three columns.

For EBR0 in MIB_EBR1, the MODE variants `DP16KD` and `PDPW16KD` have the same pattern. Another fragment of the same block tells them apart. Similarly, parts of DATA_WIDTH_B distinguish only classes of widths, not every value. The summary stores both the local matches and the result of merging the patterns of the whole field.

## WID and separate memory initialization

The WID word contains nine logical bits. For 156 EBRs it is written as one fragment, for 52 EBR3 as two: five bits are in one tile and four in the neighboring one. Both descriptions have width nine; missing positions are marked `-`.

In config_word_layout.csv, the index `logical_bit_lsb0` corresponds to the order of the groups in WordSettingBits, starting from the lowest logical bit. For each position, the CRAM constraints for writing a one are stored. An empty group has the status `no_bits_in_fragment` and an empty list of constraints; it is not declared a constant. For all 208 WID, exactly nine observable positions were verified after merging the fragments.

`nextpnr::write_bram` writes WID after reversing the order of the nine bits. So the `.bram_init` identifier cannot be obtained by simply reading the printed WID word without accounting for this conversion. This conversion is implemented; all 512 values were verified separately, and the binding of three images was verified when loading the corresponding `.config` and `.bit`.

EBR data is transferred separately from CRAM. For ECP5, libtrellis stores 2 048 nine-bit values per image; the write command carries them in groups of eight in a 72-bit frame. The INITVAL conversion from nextpnr uses 18 useful bits from each 20-bit chunk and splits them into two nine-bit values. This follows from `Bitstream.cpp` and `write_bram`, not from the configuration fields of the tiles.

The decoder distinguishes a disabled block, an unknown mode/WID, repeated WID owners, a missing INIT, and a wrong image length. The parser rejects values of more than nine bits. [EBR addressing](EBR_ADDRESSING.md) now links this image with the six port widths, the nine-bit masks, and the INITVAL conversion; six Yosys probes verified all INIT words. Unknown states, the bounds of valid hardware IDs, and partial streams remain open. For `.bit`, libtrellis does not provide a mask of the words written: the array length of 2 048 does not certify that the INIT write is complete. The rules and limits are listed in CONFIG_DECODER.md.

## DSP: shared fields and state

Additional information is collected in [DSP_INTERNALS.md](DSP_INTERNALS.md): the exact D/B interfaces from the Diamond reference, the split of C and R, the CFB mode, the chain directions, 34 repeating groups of identical control encodings per block, and parameters without a field of the same name. The research tables keep bit-level evidence; the owners of each FF are not yet established.

The MULT18 and ALU54 settings are bound through a group of 18 tiles MIB_DSP0–8/MIB2_DSP0–8. The left slice uses columns 0–4, the right one 4–8; the shared column may contain the fields of both. The settings `DSP_LEFT.CIBOUT` and `DSP_RIGHT.CIBOUT` are tied to the slices. The CIB routing fields in the neighboring CIB_DSP stay in the general inventory catalog and are not declared internal settings of the multiplier.

For 1 742 composite fields, the defaults of individual fragments differ. For example, MULT18_0.MODE occurs with the defaults NONE and MULT18X18D. This is not a ready conclusion about an error in the database: the fragments set different parts of the mode, and sometimes contain empty conditions. The working decoder merges the CRAM constraints across fragments and keeps the defaults as provenance; a default is not substituted for an unrecognized result. The semantics of the shared internal state still needs the context of the mode.

Of the 6 475 shared bits, 5 694 occur in MULT18 and ALU54 fields, 780 in several MULT18, and one belongs to the previously found OSC/USRMCLK. Example: CRAM F383B122 is described at the same time as ALU54_3.REG_INPUTC1_CLK and MULT18_1.REG_INPUTC_CLK in the first DSP group. Both representations are kept. Such a coincidence shows that the configuration control is shared; by itself it does not yet prove which physical register or mux they control.

Pre-adders, 9-bit modes, shared output registers, and cascades require a node-by-node description. From the [Lattice guide](https://www.latticesemi.com/view_document?document_id=50469), it is already known that part of the state is shared. The current interface hierarchy does not assign MULT18 and ALU54 independent copies of these registers.

## Service and additional DSP sites

The additional DSP sites are now researched in [DSP_ROUTING.md](DSP_ROUTING.md): the PRADD9/18, MULT9, ALU24 interfaces, binding candidates, PO→A paths, and control bits. Separately accounted for: 306 overlaps of fixed and programmable sources per block and 24 boundary namespaces. Executing these records requires mode conditions.

[unmodelled_resource_evidence.json](inventory/LFE5U-85F/unmodelled_resource_evidence.json) contains source references, the wire names found, and references to the corresponding database rows. The search uses separate identifiers, so SEDSTART does not become evidence of an interface of the START site, and PORT does not become POR.

| Family | Observed wire names | Separate fields with the family name |
|---|---:|---:|
| CIBTEST | 64 | 0 |
| PRADD9 | 86 | 0 |
| PRADD18 | 158 | 0 |
| MULT9 | 125 | 0 |
| ALU24 | 148 | 0 |

For CLKTEST, BREFTEST, PVTTEST, START, BCINRD, and BCLVDSO, site references were found in the types used, but not interfaces under these names. POR is represented by a tile with an empty database. This is the limit of the name search, not proof of the absence of hardware or hidden settings. The Trellis description links CIBTEST/CLKTEST/PVTTEST with factory functions, but does not define their executable behavior.

PRADD9/18 and MULT9 also have mux descriptions; ALU24 has fixed connections. Therefore the absence of a BEL or of its own enum prefix does not mean the absence of an internal structure. The names and connections are stored for later recovery. Interface directions, physical instances, and modes are not invented from them for now.

## Checks and the next result

Fifteen portable catalog checks pass. Added are checks of the ranges of all EBRs, of the split WID, of local and composite MODE, of the DSP hierarchy, of the shared control bits, and of the limits of the information about service sites. The native check of all new interfaces also passed.

Regeneration gave byte-identical 15 files, including decode_catalog.json. The CSV/JSON data with and without the native check matched; only the record of the check performed in the report differs. Verified: the rejection on a stale checksum before any files are written, the detection of opposite requirements on one enum bit, and the refusal to merge words of different widths.

```sh
python3 tools/bind_trellis_sites.py --native-probe target/trellis_resource_probe
python3 -m unittest discover -s tests -p 'test_*.py' -v
```

Decoding from CRAM and the INIT link were verified on a composite example with three EBRs and MULT18 registers; details and tests are described in CONFIG_DECODER.md. The current priority is collecting information about the internal structure and the operating rules; the questions about the owners of the shared DSP state are listed in DSP_INTERNALS.md. [EBR_ADDRESSING.md](EBR_ADDRESSING.md) contains the confirmed views of the shared array, the write modes, and the limits of the sources. [EBR_CONTROL.md](EBR_CONTROL.md) separates the synthesis contract for CE/reset/masks from the physical behavior and describes moving registers out into the fabric. Before EBR execution, B01–B08 remain; all of them are part of the 12 series of future experiments of the [hardware queue](HARDWARE_VALIDATION.md). Unresearched factory functions remain explicit gaps of the device passport and are not turned into silent stubs.
