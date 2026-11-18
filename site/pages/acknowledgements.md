# Acknowledgements

## How this was built

This project was developed with AI: Claude Code (Anthropic), OpenAI Codex and Google Gemini wrote
the implementation, tests and documentation, and ran the design reviews. Project direction,
technical decisions, playtesting and hardware validation remained human-guided, with comparisons
performed on a real ULX3S board.

## Open projects

This emulator exists because of the following open projects.

- [Yosys (YosysHQ)](https://github.com/YosysHQ/yosys) — synthesises the Verilog designs.
- [nextpnr / nextpnr-ecp5 (YosysHQ)](https://github.com/YosysHQ/nextpnr) — places and routes designs; supplies the window's routed JSON and timing reports.
- [Project Trellis (YosysHQ)](https://github.com/YosysHQ/prjtrellis) — documents ECP5 bitstreams; supplies the image reader's libtrellis, ecppack/ecpbram and the hash-checked [prjtrellis-db](https://github.com/YosysHQ/prjtrellis-db) database.
- [Radiona / ULX3S](https://radiona.org/ulx3s/) — provides the open [KiCad board design and photos](https://github.com/emard/ulx3s) behind board wiring, the 3D model and visual checks.
- [EMARD's ULX3S constraints](https://github.com/emard/ulx3s/blob/v3.0.8/doc/constraints/ulx3s_v20.lpf) — provide the pin constraints the examples derive from.
- [PicoRV32](https://github.com/YosysHQ/picorv32) — Claire Xenia Wolf's soft CPU runs in the console and gallery computers.
- [SERV](https://github.com/olofk/serv) — Olof Kindgren's bit-serial core runs `serv_shell`.
- [OSS CAD Suite](https://github.com/YosysHQ/oss-cad-suite-build) — supplies the pinned binary toolchain for CI and the install guide.
- [openFPGALoader](https://github.com/trabucayre/openFPGALoader) — loads the ULX3S SRAM.
- [egui and eframe](https://github.com/emilk/egui) — provide the window's controls and application framework.
- [wgpu](https://github.com/gfx-rs/wgpu) — draws the die map and board on the GPU.
- [winit](https://github.com/rust-windowing/winit) — provides native windows and input events.
- [Rust and its crates](https://github.com/rust-lang/rust) — provide the compiler, standard library and dependencies; full crate credits are in the packages' `THIRD_PARTY_NOTICES.txt`.
- [Inter](https://github.com/rsms/inter) — supplies the 3D board's chip markings.
- [Ubuntu font](https://design.ubuntu.com/font) — supplies proportional text bundled by egui.
- [Hack](https://github.com/source-foundry/Hack) — supplies monospace text bundled by egui.
- [Noto Emoji](https://github.com/googlefonts/noto-emoji) — supplies emoji glyphs bundled by egui.
- [emoji-icon-font](https://github.com/jslegers/emoji-icon-font) — supplies additional glyphs bundled by egui.
- [KiCad](https://www.kicad.org/) — exports the board layers through `kicad-cli`.
- [Blender](https://www.blender.org/) — builds and exports the 3D ULX3S model.
- [librsvg](https://gitlab.gnome.org/GNOME/librsvg) — rasterises board layers and the icon through `rsvg-convert`.
- [Boost](https://github.com/boostorg/boost) — supplies C++ libraries used by libtrellis.
- [Clang / LLVM](https://clang.llvm.org/) — compiles fast-mode C and the native image reader.
- [Verilator](https://www.veripool.org/verilator/) — runs the gallery's RTL reference simulations.
- [Icarus Verilog](https://steveicarus.github.io/iverilog/) — runs FIR response checks and the DSP reproducer; supplies PicoRV32 reference traces.
- [Pillow](https://python-pillow.org/) — composites board textures and visual QA sheets.
- [NumPy](https://numpy.org/) — processes board texture arrays and rendering calculations.
- [mdBook](https://github.com/rust-lang/mdBook) — builds the documentation site and supplies its theme assets.

- [Zig](https://ziglang.org/) — supplies the C compiler bundled for fast mode.
- [GHDL](https://github.com/ghdl/ghdl) and its [Yosys plugin](https://github.com/ghdl/ghdl-yosys-plugin) — synthesise VHDL-2008 projects.
- [slang / yosys-slang](https://github.com/povik/yosys-slang) — supplies the explicitly selected SystemVerilog frontend.

## Build and research tools

- [Python](https://www.python.org/) — runs the build, packaging, research and reference-check scripts.
- [CMake](https://cmake.org/) — configures the native Trellis library build.
- [Ninja](https://ninja-build.org/) — runs the native library build when selected by the packaging helper.

## Documentation theme

The ledger also records these projects bundled with mdBook.

- [Open Sans](https://github.com/googlefonts/opensans) — is bundled as a documentation theme font.
- [Source Code Pro](https://github.com/adobe-fonts/source-code-pro) — is bundled as a documentation code font.
- [highlight.js](https://github.com/highlightjs/highlight.js) — highlights documentation code blocks.
- [Font Awesome](https://github.com/FortAwesome/Font-Awesome) — supplies documentation theme icons.
- [clipboard.js](https://github.com/zenorocha/clipboard.js) — supplies documentation copy controls.
- [mark.js](https://github.com/julkue/mark.js) — highlights matches in documentation searches.
- [elasticlunr.js](https://github.com/weixsong/elasticlunr.js) — supplies the documentation's client-side search.
- [Ace](https://github.com/ajaxorg/ace) — is bundled for editable documentation code blocks.
- [Ayu](https://github.com/dempfi/ayu) — supplies a documentation colour theme.
- [Tomorrow Night](https://github.com/chriskempson/tomorrow-theme) — supplies documentation syntax colours.
- [color-themes-for-highlightjs](https://github.com/jmblog/color-themes-for-highlightjs) — supplies documentation syntax colour themes.
- [Base16](https://github.com/chriskempson/base16) — supplies documentation syntax colour themes.

See [licenses/README.md](../../licenses/README.md) for licences and evidence.
