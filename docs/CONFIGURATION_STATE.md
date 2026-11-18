# Configuration image and separate chip state

This describes the second code milestone and its 35 checks. It was followed by the [LUT/FF event kernel](EVENT_KERNEL.md), so far with an explicitly given connection plan. `load` still only prepares a snapshot; the old `run` has not been moved to the new path yet.

The second milestone of the main code, October 22, 2026. To the permanent `DeviceModel` were added an immutable `ConfigurationImage`, a bound `ConfiguredDevice` and a separate mutable `DeviceState`. The `load` command prepares them from a real `.bit` or `.config`. Events, startup and behavior models are not yet executed through this layer.

## What is loaded now

The new C++ bridge `trellis_image_reader` reads the source bytes through stdin and calls libtrellis directly. For `.bit` the intermediate conversion `Chip → .config → Chip`, which lost the service data, is no longer needed. Kept are: the source bytes/their SHA-256, CRAM, all decoded EBR INIT, the metadata comments as bytes, and four Chip values: CTRL0, CTRL1, USERCODE and the SED reference.

These four values have the origin **"native parser state"**. Chip holds the constructor defaults; the new bridge does not prove that each corresponding command is present in the source stream. Therefore `parser_defaults_may_apply` is explicitly set for `.bit`. The source stream is kept whole for further parsing, including the commands that the library consumes without full controller state.

For `.config` the four service values are absent (`None`) and are not replaced by Chip defaults. The source `.sysconfig`, `.comment` and `.variant` are kept separately. The mere presence of a directive does not declare execution of the configuration controller. The library rejects formats with unsupported commands; this is not a full SPI/JTAG controller or support for arbitrary partial reconfiguration.

The image has two different hashes: of the source bytes and of the decoded content. The fingerprint of the bound configuration also accounts for the architecture manifest and the composition of the merged catalog. The working state checks this binding; the same source file name, or the same bytes with a different decoding/catalog, do not give a false match.

## Fields and routes

Five research catalogs are merged with source verification through [configuration_manifest.json](inventory/LFE5U-85F/configuration_manifest.json): the earlier EBR/DSP/system, IOLOGIC, the full PIO/banks, PLL/DCC/DCS/OSC and DQS. The result is **28 810 composite fields for 1 371 resource/bank records**. Exact duplicate fragments are removed; contradictory records from the same source are rejected. Different fragments of one field are kept together, including differing defaults.

The CRAM sizes, the membership of resources in the architecture and the placement of each fragment's bits within its tile are checked. Bank records are linked to the existing BANKREFs. Ambiguous, partial, unobservable and non-matching encodings remain so in the report. They do not become an allowed mode by default. This is not the completion of the mode interpretation of all families.

In addition, the Rust database parser keeps the bit groups of all words. For the **83 640 LUT INITs** they are applied directly to CRAM with the correct polarity and bit order. This path is also available through `ConfiguredDevice::word`; empty and divergent groups give X. The full composite decoder of all scalar PLC/FF fields is still to be connected; their count is not added to the number 28 810.

`raw_mux` returns the matching patterns and the candidates that are maximal by inclusion. Incomparable patterns remain alternatives, even if one has more bits. Density exceptions are taken into account; candidates that fall outside the die are marked. The result has `requires_mode_interpretation`: the shared INJECT/F bits and the fixed/mux bypasses do not automatically become active physical paths.

## Working storage

| Object | State after preparation |
|---|---|
| 83 640 LUT | One array of 16 logical bits from CRAM each, 1 338 240 bits in total |
| 83 640 FF | X until an explicitly established startup procedure; REGSET is not accepted as evidence of PUR |
| 208 EBR | One common 2 048×9 array each; INIT is applied only with an unambiguous full binding |
| Wires | No value until observed/set; this differs from an established Z |

A LUT and its future LUTRAM mode share one storage; RAMW does not create a second copy of the array. EBR keeps the ninth bit and a single physical representation regardless of the future port width. A missing/incomplete INIT, an ambiguous MODE or a repeated WID leave the memory X. Unbound source INITs remain in the immutable image in this case.

For `.bit` the limitation `native_decoded_without_write_mask` is kept: libtrellis does not report the mask of the EBR words actually written. The working snapshot uses the unambiguously bound decoded values, without presenting this as a recovered sequence of physical writes. For text, `explicit_text_words` is marked. Checking transport loading and partial writes remains a separate stage.

The state write methods currently change the raw storage. They do not implement CE/reset/GWDIS, memory ports, collisions or edges. The changes do not affect `ConfigurationImage`, `DeviceModel` or another independently created `DeviceState`. `load --output` saves a summary; a full save/resume of the working state is not implemented yet.

## Running

```sh
python3 tools/build_image_reader.py
python3 tools/build_configuration_manifest.py
cargo run --release --locked -- load test.config --output target/prepared.json
cargo run --release --locked -- load target/toolchain-update/smoke/top.bit
```

The new path does not need Python/pytrellis at load time; it needs the built bridge and a compatible libtrellis. The bridge path is set by `--image-reader`/`ECP5_IMAGE_READER`, the database path by `--db`/`ECP5_DB`, and the research tables by `--architecture`. The build helper is designed for the current macOS/Homebrew installation and does not modify the Trellis checkout. The old commands `decode` and `run` keep the previous path: the new `load` does not declare that they were moved to the event kernel.

The [`.config` summary](inventory/LFE5U-85F/prepared_text_summary.json) and the [`.bit` summary](inventory/LFE5U-85F/prepared_bit_summary.json) are saved for one example. The CRAM, the set of fields and the initial memory data match; the service information and the source fingerprints differ, as expected.

## Checks and the next milestone

18 unit tests, five portable architecture checks, eight new image/state checks and four explicitly run integration checks passed — **35 tests**. Clippy passed with no warnings, formatting passed. The [machine report](inventory/LFE5U-85F/configuration_state_validation.json) records the scope of the check and the hashes of the sources. The main new checks:

- Missing text registers, native defaults provenance, comments and the exact source bytes.
- Independence of two working states from the image and from each other; binding to the content/catalog.
- The ninth EBR bit, a missing INIT, a repeated WID, empty and contradictory bit groups.
- An ambiguous mux and conflicting catalog fragments.
- An ordinary and a compressed `.bit` with the asymmetric LUT INIT 0xa15c and EBR INIT: the CRAM/INIT and the initial storage matched those of `.config`. A synthetic tail, for the parser only, confirmed that all four service values are preserved; it was not sent to the board.

Also passed again: the full architecture check of 197 408 clock connections, the earlier native EBR/DSP/INIT check and the old counter with 65 538 observations. The test of the old external blink `.bit`, which requires ECP5_TEST_BIT, was not run separately. There are no hardware results for the new models.

The next milestone is virtual time, the queue and the event phases, driver resolution, and execution of LUT/FF/carry/PIO through directed paths. Startup/GSR, shared controls and physical uncertainty keep their research IDs. Keeping a block's settings and allocating its memory do not count as implementing its behavior.
