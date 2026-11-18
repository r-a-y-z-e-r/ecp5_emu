# Live values in source tabs

Status: design only; the editor feature is not implemented. The design incorporates the publication regression test in `tests/published_values.rs`. Existing behavior is cited by file and function; future UI/index work is described as a requirement. No runner change is in scope.

## Experience

Hovering a signal identifier in a project source tab should show its last published value, labelled **at cycle N, as published**, and offer **Watch in Waves** for edge-by-edge values. An optional editor toggle, off initially, should draw values after the ends of visible assignment lines. These annotations describe the current published values of the assigned signals, not the result of evaluating that particular assignment or its right-hand expression.

Annotations are presentation only: they must not enter the text buffer, affect saving, mark a tab dirty, or change character coordinates. The existing editor stores editable text separately from its galley and paints its gutter using galley rows; that is the integration point for pointer hit testing and inline painting. It currently probes clicks/cursor moves, not signal-value hovers, and its layout function supplies line highlighting rather than HDL syntax analysis. Evidence: [gui/src/project/editor.rs](../../gui/src/project/editor.rs), `Editor::show`, `build_layout_job`, `Tab::dirty`, `Tab::save`, `line_column`.

## 1. Published values and name resolution

Use only `Shared::lit` as the value source. No passive hover or inline annotation should enqueue runner work, add a watch, or change `src/live.rs`.

The exact fast-mode call chain is `work::publish` → `FastSim::step` → loaded C symbol `fast_step` → full `settle(v)` → `converge(v)` → `read_values` → `FastSim::read`. This settles at the current time; it does not advance the board clock. The publication then assigns `lit`, `cycles = done / 2`, `edges = done`, and increments `published` under one shared mutex. Evidence and exact source excerpts:

1. [src/live.rs](../../src/live.rs), `work`'s `publish` closure, lines 1825–1831:

   ```rust
   // Gates that only registers with an enable read may be left from an
   // earlier cycle: settle everything before reading the LUT outputs.
   fast.step()?;
   // All read before taking the lock, which the window waits on: a big
   // design has a million wires and more.
   let values = read_values(fast, &value_reader);
   let lit = read_values(fast, &lit_reader);
   ```

2. [src/fast.rs](../../src/fast.rs), `open`, line 328, binds `step` to `symbol("fast_step")`. `FastSim::step`, lines 1283–1285, calls:

   ```rust
   let status = unsafe { (self.loaded.step)(self.values.as_mut_ptr()) };
   self.finish_dtrs(status)?;
   self.track_plls()
   ```

3. [src/kernel/fast.rs](../../src/kernel/fast.rs), `FastKernel::program`, lines 1452–1464: the `cone` closure walks the original `lines` in dependency order and appends a line when an input is reached. Its full-settle seed is exactly:

   ```rust
   let settle = cone(&(0..slots).collect::<Vec<_>>());
   ```

   This is built before the on-demand classification at lines 2380–2418 and before group folding via `collapse`/`collapse_cones` at lines 2699–2727. Therefore the full settle includes those original folded-cone and on-demand gates, rather than only the reduced edge-fresh group settles. The generated-C template at lines 3189–3190 and 3235–3237 contains (doubled braces and interpolation markers are literal Rust template text):

   ```text
   static void settle(u8 *restrict v) {{
   {settle}}}
   int fast_step(u8 *restrict v) {{
   {clear_error}  settle(v);
   {check_error}  return converge(v);
   ```

   `settle` is the function that recomputes every such gate for a publication. `settle_demanded` at lines 3197–3198 serves simultaneous captures; it is not the full-publication entry point. `converge` handles captures/resets and returns once no edge/reset work remains (same generator, lines 3216–3231).

The event kernel supplies startup state, not a separate `Shared::lit` publication path. [src/live.rs](../../src/live.rs), `start_fast`, lines 1221–1225, runs whole warm-start cycles, with each half-cycle executing:

```rust
kernel.schedule_batch(time, &[(clock, Logic::from_bool(done % 2 == 1))], None)?;
kernel.run_until(time)?;
```

The event equivalent is `Kernel::run_until` → `Kernel::advance` → `Kernel::settle` → `Kernel::settle_inputs` → `Kernel::settle_combinational`, repeating sequential/combinational waves until quiescent. [src/kernel/mod.rs](../../src/kernel/mod.rs), `run_until`, line 1224, calls `self.advance(time, events)` for every queued timestamp through `until`. `advance`, lines 1270–1271, ends its stimulus wave with:

```rust
self.publish(updates, Phase::Stimulus)?;
self.settle()?;
```

`Kernel::settle` calls `self.settle_inputs()?` (line 1661) and returns only when the register, clock-group, LUT-write, EBR, multiplier, IOLOGIC, DCS, and DTR dirty sets are empty (lines 1666–1675). [src/kernel/system.rs](../../src/kernel/system.rs), `Kernel::settle_inputs`, line 194, calls `self.settle_combinational()?`; [src/kernel/mod.rs](../../src/kernel/mod.rs), `settle_combinational`, lines 1592–1654, drains dirty gates/multipliers and publishes their delta waves. Initial construction also settles: `Builder::build` resolves initial drivers, dirties all combinational gates, and calls `kernel.settle()?` before `collapse_buffers` (lines 941–944, 1003–1004). Warm start hands this state to `FastSim::start`; `work`'s first value publication still uses the fast closure above.

The UI promises **0/1/x**, not four-state exactness. `FastSim::read` maps X, Z, and missing program slots to byte 2 ([src/fast.rs](../../src/fast.rs), `FastSim::read`); label `x` **unknown, high-impedance or not available here**. Do not infer which cause applies. Missing name/bit resolution remains a separate unavailable result (`?` for a bus gap). Preserving Z would require a separate runner change and is out of scope.

**Measured limit:** a full settle does not establish event-kernel equality for every overlay entry. In both Life fixtures, an unused carry-helper sum output is Boolean 0 in fast mode and X in the event kernel, even after the full settle. The verified exception is **event X versus a fast Boolean on a net with no readers**, not an unsettled folded cone. The regression now allows only that semantic class and fails on any other difference; it does not whitelist coordinates or carry-cell names. All measured folded and on-demand entries agree.

**Hover reachability:** neither differing net can be reached by any HDL name in these fixtures. The test resolves every HDL name (including hidden/generated aliases and package ports) through `Names::overlay_nets` and finds no target for either net; this is stronger than inspecting a ROUTING string. `Collected::build` uses that same resolver, so a name-only hover cannot reach them and needs no special value mask for these helpers. An unresolved identifier still gets the no-net-on-chip message. See `check` in [tests/published_values.rs](../../tests/published_values.rs), `Names::resolved` and `Names::overlay_nets` in [src/names.rs](../../src/names.rs), and `work`'s source-index construction in [src/live.rs](../../src/live.rs). `src/live.rs` stays untouched; universal overlay equality is not claimed.

Read a coherent tuple of design identity, `published`, `cycles`, `edges`, status, and requested `lit` entries under one lock. Never label an old value with a newer cycle. Retain `edges` internally because two half-cycles have the same integer cycle; include the edge/half-cycle in tooltip detail when needed to distinguish snapshots. This follows the actual `done / 2` representation in `work::publish` ([src/live.rs](../../src/live.rs)).

Resolve the hovered identifier through the cross-probing index, by name only and within the current project-relative file. Do **not** call `Index::resolve` or `Index::probe` for hover: both can use location fallback after a name miss. Add a small accessor exposing only the private per-file name map, ideally distinguishing absent names, known names with no targets, and resolved names. `Collected::build` already preserves empty entries for known unrouted names, adds bus-base aliases, and sorts/deduplicates net IDs. Evidence: [src/sources.rs](../../src/sources.rs), `Collected::build`, `Index::resolve`, `Index::probe`. This is a proposed accessor, not an existing API.

For each resolved net, the verified map path is:

```text
overlay.pieces_of(net).first()
    -> overlay.pieces[piece_index].value
    -> published lit[value_index]
```

`Piece::value` is an index, not a logic value. This is exactly the wire selection used by [gui/src/waves.rs](../../gui/src/waves.rs), `Waves::add_from`; the underlying API is [src/fabric_overlay.rs](../../src/fabric_overlay.rs), `Overlay::pieces_of`. Missing pieces or out-of-range value indices must produce an unavailable result, never a nearby entry. `Names::overlay_nets` maps a routed source through `Overlay::value_of` and `overlay.nets`; package pins may resolve to multiple overlay nets ([src/names.rs](../../src/names.rs)). Preserve qualified alternatives; do not pick an arbitrary instance or pin target.

## 2. Freshness and Waves

The hover is a snapshot, not a history. It says **at cycle N, as published**: the state at publication, after inputs applied at that instant. **Watch in Waves** is the explicit action for observing subsequent edges. At the same edge and input state, a binary bit Waves sampled equals the hover; make no unconditional promise for different instants, high-impedance encoding, or a bus number wider than Waves' current formatter supports.

The distinction in freshness is confirmed: [src/kernel/fast.rs](../../src/kernel/fast.rs), `FastProgram::fresh`, means a signal stays current after every edge. [src/fast.rs](../../src/fast.rs), `FastSim::run_edges_watched` and `FastSim::sampled`, mark unmaintained watched columns `NOT_SAMPLED`. [src/live.rs](../../src/live.rs), `Sampled::run_with` and `Sampled::deliver`, preserve missing sampling information. A later full settle makes a folded signal readable for the snapshot; it does not retroactively sample its Waves row. Such folded rows remain `NOT_SAMPLED`, including at a published edge, rather than reporting a different number. The runner applies inputs/peripheral answers after collecting edge rows and before publication (`work`, `apply_input`, `answer`, same file), which is why the input-state qualifier matters.

The hover action should pass its exact name-only resolved targets into the existing watch mechanism. It must not re-resolve via the click probe path. [gui/src/app.rs](../../gui/src/app.rs), `App::watch_probed`, currently adds previously probed nets with `Waves::add_net_from` and sends `Waves::command`. That provides the integration pattern, but ordinary hover must not call `App::probe_source`, which also changes map selection and probe state. A watch can trigger a background program build for folded nets: [src/live.rs](../../src/live.rs), `WatchPlanner::on_watch`, `WatchPlanner::missing`, `start_watch_build`. Only the explicit action incurs that cost.

## 3. States and invalidation

| State | Required hover and inline behavior | Existing evidence |
| --- | --- | --- |
| Paused with a valid publication | Show the settled 0/1/x snapshot **at cycle N, as published**, including the initial paused snapshot. | `work`, initial `publish` call and `Command::Pause` branch, [src/live.rs](../../src/live.rs). |
| Running | Show the last publication and its cycle; refresh when `published` changes. Target about 30 ms at full speed and per batch at slow rates, with no every-edge guarantee. | `work::publish`, `PUBLISH`, pacing/publish branches, [src/live.rs](../../src/live.rs). |
| Back/seek pending | Suppress old numbers and show **seeking…** until a new publication at the GUI's target cycle is taken after sending the command. No old inline numbers during this interval. | `App::send`, [gui/src/app.rs](../../gui/src/app.rs); `seek`, `work` Back/GoTo branches, [src/live.rs](../../src/live.rs). |
| No emulated design | No value; explain **Emulate this project to see values**. | `App::has_design`, [gui/src/app.rs](../../gui/src/app.rs); `ProjectApp::probe` checks the emulated project root, [gui/src/project.rs](../../gui/src/project.rs). |
| Loading / no first publication | No value; explain that the design is loading or values are not available yet. Values become eligible at the first valid publication. | `work` exposes `ready` before indexing and before its initial `publish`, [src/live.rs](../../src/live.rs). |
| Source index pending | No value; explain **Source index is being prepared**. Ready alone is insufficient. | `App::poll_sources`, [gui/src/app.rs](../../gui/src/app.rs); `ProjectApp::probe`, [gui/src/project.rs](../../gui/src/project.rs). |
| Load failed | No value; show the load failure reason. | `App::state`, [gui/src/app.rs](../../gui/src/app.rs); `ProjectApp::probe`, [gui/src/project.rs](../../gui/src/project.rs). |
| Source edited since build | Keep the existing stale-source warning; show values only for identifiers that still resolve in the running design's index. Never imply the edited code is being simulated. | `ProjectApp::stale_warning`, `source_warning`, [gui/src/project.rs](../../gui/src/project.rs). |

The GUI owns seek suppression. Before sending Back/GoTo, retain the current design identity and publication counter, compute the target cycle (Back clamps to `earliest`; GoTo follows the runner's target semantics), and hide values. Release only on a different publication at that target, taken after command dispatch; an unrelated new publication at another cycle is insufficient. A superseding seek replaces the pending target. This is the selected GUI contract, not a runner acknowledgement protocol; no runner request IDs or seeking fields are added. Existing hooks: `App::send` ([gui/src/app.rs](../../gui/src/app.rs)), `Shared::earliest`, `work` Back/GoTo branches, and `seek` ([src/live.rs](../../src/live.rs)).

`PUBLISH` is 30 ms; batches and GUI frames can skip intermediate edges/publications. The `slow` branch publishes per loop batch at rates up to 1000 c/s, but `to` can contain several due edges. Paused startup may already be at a nonzero warm-start cycle. Evidence: `work` pacing, chunk adaptation and `slow`; `start_fast` ([src/live.rs](../../src/live.rs)); `Taken::take` ([gui/src/app.rs](../../gui/src/app.rs)).

The existing warning is `built from an older <file>: locations may be off`. It detects dirty editor buffers or a source mtime newer than the routed JSON's mtime. It is not a content-hash guarantee. Evidence: `ProjectApp::stale_warning`, `source_warning` ([gui/src/project.rs](../../gui/src/project.rs)); `Names::load_with_sources` sets `built_at` ([src/names.rs](../../src/names.rs)).

On rebuild, retain the running design's identity until it is replaced; a newly built report is not the running source index. On replacement, discard all target/value caches and bind to the new `Static` and index even if publication numbers repeat. Existing replacement resets shared/view state in `App::switch_ready` ([gui/src/app.rs](../../gui/src/app.rs)); `ProjectApp::ui` explicitly notes that build reports can be newer than the emulated design ([gui/src/project.rs](../../gui/src/project.rs)).

## 4. Identifiers, buses, and unavailable names

Hover recognition must use an editor-side Verilog lexical filter: signal identifiers and supported constant bit selects only. Keywords, comments (including multiline comments), strings, and sized/based numeric literals have no value hover. Use the current editor buffer and one-based character coordinates, as `Editor::probe_at` and `line_column` do ([gui/src/project/editor.rs](../../gui/src/project/editor.rs)). Existing `sources::identifier_at` recognizes names and nonnegative constant selects but lacks those exclusions; it cannot be reused unchanged ([src/sources.rs](../../src/sources.rs), `identifier_at`).

For a valid signal identifier with no chip target, use the deliberately non-diagnostic message:

> `x` has no net on the chip: optimised away, a constant, or inside a cell

Do not claim which cause applies, and never substitute a neighbour's value. Existing `Index::probe` has a similar message for known empty names, but an entirely removed name is absent and can fall through to locations. Evidence: `Collected::build`, `Index::probe`, `Index::resolve` ([src/sources.rs](../../src/sources.rs)); `source`, `Names::overlay_nets` ([src/names.rs](../../src/names.rs)). Apply the proposed message to valid signal tokens after name-only lookup, including misses in edited source.

Bus formatting requirements:

- Preserve bit positions from `x[i]`, with highest numeric index displayed first; never use sorted net IDs as bit order. `Collected::build` sorts IDs, whereas Waves explicitly parses and orders bit names in `Waves::lines` ([src/sources.rs](../../src/sources.rs), [gui/src/waves.rs](../../gui/src/waves.rs)).
- Use a vector of bit states and nibble-by-nibble hex conversion, with no `u64` limit. Group displayed hex digits for readability; suppress decimal above 64 bits or whenever any bit is nonbinary or missing. Proposed decimal interpretation is unsigned unless a later design adds signedness metadata.
- A nibble containing a published x renders `x`, labelled **unknown, high-impedance or not available here**. A nibble containing a missing bit renders `?`; missing takes precedence over x. There is no separate z rendering with this value source.
- An expected bit with no routed target is `?`, never zero. List missing names explicitly, for example `x[6], x[2]`. Missing is different from a resolved entry containing X. Pad a partial most-significant nibble only outside the known width; never pad an unresolved bit as zero.

The implementation must retain each name's bit layout as nextpnr lists the netname bits, rather than just a sorted/deduplicated list of overlay targets. Preserve per-bit names/indices and instance prefixes before resolving routes. Within the found indexed range, unresolved gaps are `?`; invent nothing outside it. The hover says **bits k..m found**, so partially or entirely optimised-away declaration widths are not implied. Multiple instances produce one row per instance prefix, never a preferred instance. Several physical targets for one bit stay explicit alternatives if they cannot be proved equivalent.

This is additional index work: `Collected::from_top` currently filters JSON bits to numeric IDs and `Collected::build` merges local suffixes and bus bases into target vectors; neither preserves the required layout ([src/sources.rs](../../src/sources.rs)). Any-width formatting is also new: `Waves::line_value` accumulates a whole bus into `u64` and collapses exceptional bits to a single error state; it cannot be reused as this formatter ([gui/src/waves.rs](../../gui/src/waves.rs)).

## 5. Cost and ownership

Per frame, the feature should inspect only the hovered name's bits and, with the toggle enabled, assigned names on visible lines. Cache name-to-wire mappings independently of value formatting. Reformat retained visible targets only when `published` moves. New hover/viewport targets can be filled from the current snapshot on demand; do not repeatedly scan all project names or all lines for values. Text edits, scrolling, tab changes, design/index replacement, and toggle changes invalidate the relevant target set; these are not new simulation samples.

The existing map already copies all published `lit` values only when `published` changes: `Taken::take` ([gui/src/app.rs](../../gui/src/app.rs)). Reuse that snapshot if available, or expose a sparse shared read for the project editor. `App::poll_sources` currently supplies indices while the project view is visible, not a value snapshot; the normal `App::ui` path takes map values. Ensure the source editor reads while the chip tab is hidden. Do not add a second full-vector copy for this feature, and do not hold the shared lock through tokenization, formatting, painting, or file IO.

Inline targets require assignment-aware tokenization of the current buffer. Only annotate confidently recognized left-hand names, including constant selects; skip unsupported constructs rather than guessing from the whole line's source span. Multiple assigned names may share an end-of-line annotation. Cache syntax/target discovery on buffer changes; limit value reads and painting to visible galley rows. The existing index's driven spans come from cell-output origins, not an assignment parser: `Collected::from_top`, `Collected::build` ([src/sources.rs](../../src/sources.rs)).

## 6. Proposed files and recording

Proposed responsibilities:

| File | Proposed responsibility |
| --- | --- |
| `gui/src/project/editor.rs` | Pointer-to-token handling, lexical exclusions, assignment targets, visible inline painting, toggle, state/message rendering. |
| `gui/src/project.rs` | Project/file identity, existing stale warning, current buffer, editor snapshot requests, scripted hover dispatch. |
| `gui/src/app.rs` | Read-only access to coherent published values and the source index while the project tab is active; cache lifecycle, seek suppression, explicit watch action and record notes. |
| `src/sources.rs` | Name-only accessor and per-name nextpnr bit layout, preserving bit indices/gaps and qualified instance prefixes. The measured unconnected helper sums have no name target and require no masking metadata. |
| `gui/src/project/script.rs` | Add the `hover FILE LINE COL` command and parser coverage. |
| `src/live.rs` | Untouched. No extra settles, probes, sampling, watches, or publication work for passive values. |

For window records, add:

```text
# value <file:line:col> <name> = <value> @ <cycle>
```

Use the exact displayed value and its captured cycle. Emit on a changed hover result/publication, not every redraw; unavailable states need explanatory notes without inventing numeric values. The existing window writer adds `# ` in `Trace::note`; pass it `value ...`, without a second prefix. `App::probe_note` shows how source notes reach window records and the project log, and `ProjectApp::poll_probes` forwards project notes. Evidence: [gui/src/trace.rs](../../gui/src/trace.rs), `Trace::note`; [gui/src/app.rs](../../gui/src/app.rs), `App::probe_note`; [gui/src/project.rs](../../gui/src/project.rs), `ProjectApp::poll_probes`.

Add script command `hover FILE LINE COL` with the same buffer, lexical checks, name-only lookup, snapshot, and output as a real pointer hover. Coordinates are one-based characters; a relative FILE may contain spaces. The existing project script syntax includes a leading delay, so an invocation would be `0 hover rtl/top.v 12 8`. Current `source FILE LINE COL` is a click probe, not a safe alias for hover. Extend `ScriptCommand` and `parse_script` in the now-in-scope parser file. Evidence: [gui/src/project/script.rs](../../gui/src/project/script.rs), `parse_script`, `ScriptCommand::Source`; [gui/src/project.rs](../../gui/src/project.rs), `ProjectApp::poll_script`; [gui/src/project/editor.rs](../../gui/src/project/editor.rs), `Editor::probe_at`.

## 7. Planned verification

The editor/index tests below remain future work; the publication regression is implemented and reported separately below:

- Format buses of 8, 64, 65, and 128 bits, with high bits set, partial nibbles, x (including coerced X/Z/unavailable), interior/boundary gaps within the found range, and explicit missing-name lists. Check decimal eligibility, found-range labels, no invented outside bits, and no truncation.
- Name-only lookup must reject an optimised-away/absent identifier even on a line with a resolvable neighbour. Preserve known empty names. Test keywords, HDL numeric literals, comments across lines, strings, escaped identifiers, bit selects, duplicate local names, and aliases.
- Exercise every state above, including initial ready before first publish, index arriving later, dirty/saved stale source, replacement with reused net IDs, pending seek, delayed commands, failure, and project-tab-only repainting. Value and cycle must come from one publication.
- At the same edge and input state, compare individual sampled binary Waves bits against the snapshot. Check folded nets before and after a watch-program switch: an existing `NOT_SAMPLED` row must remain unsampled. Include high-impedance encoding and between-edge input cases to check the limits of the promise.
- Verify passive hover/toggle sends no runner commands, performs no extra full-vector copy, reads only requested bits, and leaves text/save behavior unchanged. Check inline placement after horizontal/vertical scrolling and multiline assignments.
- Drive pointer and scripted hover through the same path; assert the exact record format, no numeric note for unavailable results, and no per-frame duplicate notes.

Existing relevant seams are `Taken::take` and `App::watch_probed` ([gui/src/app.rs](../../gui/src/app.rs)), `Editor::probe_at` and `Editor::show` ([gui/src/project/editor.rs](../../gui/src/project/editor.rs)), `Waves::take` and `Waves::value` ([gui/src/waves.rs](../../gui/src/waves.rs)), and `Sampled::deliver` ([src/live.rs](../../src/live.rs)).

## Decided

- **Q1:** Show 0/1/x; label x “unknown, high-impedance or not available here”. No four-state promise or runner change for Z.
- **Q2:** At the same edge and input state, a sampled binary Waves bit equals the hover. Folded rows stay `NOT_SAMPLED`; the hover says “at cycle N, as published”, after inputs applied at that instant.
- **Q3:** Last-publication updates: about 30 ms at full speed, per batch at slow rates. No every-edge promise.
- **Q4:** Preserve per-name nextpnr bit layout; gaps inside the found range are `?`, nothing outside is invented, and the hover says “bits k..m found”. Show a row per instance prefix.
- **Q5:** GUI-owned target tracking: show “seeking…” until a new publication at the target cycle is taken after sending Back/GoTo. No runner change.
- **Q6:** Add the editor-side Verilog lexical filter, excluding keywords, comments, strings, and sized/based literals.
- **Q7:** Values start with the first valid publication, including the initial paused snapshot.
- **Q8:** Include `gui/src/project/script.rs` in the eventual implementation files.

## Publication regression

[tests/published_values.rs](../../tests/published_values.rs), `check`, tests both `life` and `life_full` with cone folding enabled (12 inputs, one thread, no tuning) and checkpoints 1, 7, 100, and 1000 cycles after warm start. `setup` mirrors the window's `Scope::Observed` / `Startup::Configured` Request, default Board, idle board inputs (switches off), output/peripheral probes, and overlay construction. It asserts these designs require no external peripheral answers; it adds no internal probes or watches that could make folding disappear. `advance_event` runs the same clock schedule to the same absolute timestamp. The test then calls `FastSim::step` and `FastSim::read(Reader::new(lit_signals))`, matching `work::publish` / `read_values` exactly. Clock overlay wires are mapped through `model.clock_segment`, as in `work`.

Every overlay value entry is inspected, not only source-named nets or one wire per net. Comparisons use the chosen publication encoding: event Zero/One map to 0/1; X/Z map to 2. Entries outside the event plan are explicitly counted as unavailable, not evidence of a known simulated value. A missing fast slot is compared against the event value and would fail if that value were binary. The test prints Z-coercion counts separately.

Freshness classes use public APIs: **on-demand** means stale even in a program generated with folding disabled; **folded** means fresh without folding but stale with it; **edge-fresh** means fresh in the folded program. Physical **pad-output** and **EBR-output** tags use executed resource output ports and fast-slot aliases; those classes overlap the freshness classes. **Unused carry sum** is a diagnostic tag, classified independently of observed values from nextpnr CCU2 cells with `CCU2_INJECT1=NO`, empty F, and empty FCI connections (`unused_carry_sums` in the test). That tag does not grant an exception.

The allowed-difference rule is exactly: a mapped net has **no readers**, its event value is **X**, and its published fast value is **0 or 1**. All other differences fail, with the cycle, wire, class, and both values. In particular, event Z, differing binary values, missing fast slots, or a reader prevent the exception. Counts retain actual differences and print separate `allowed` and `unexpected` totals, rather than calling allowed differences equal. Both folded and on-demand coverage are asserted nonempty. There are no coordinate/name allowlists.

Reader absence is verified from the actual compiled plan, not guessed from an unconnected nextpnr output. Since `Kernel` has no public reader accessor, test-only `PlanPrefix::load` reads the leading construction-cache fields using the exact `Core` / `Signal` / `Driver` / `Function` schema. It validates envelope magic, version 4, compile identity, length, and SHA-256, then verifies every cached signal's wire/index against the live compiled kernel. A changed/unavailable schema fails closed; the reference test requires the construction plan cache enabled. Evidence: [src/kernel/cache.rs](../../src/kernel/cache.rs), `Core`, `encode`; [src/compile/cache.rs](../../src/compile/cache.rs), `envelope`, `payload`, `write`; [tests/published_values.rs](../../tests/published_values.rs), `PlanPrefix::load`, `PlanPrefix::unread_slots`.

`PlanSignal::has_reader` checks every consumer list, including combinational gates, register data/control, clock groups, memory and other blocks, and the system flag, and cross-checks `Signal::read`. The test aggregates readers and watches across **all** signals sharing a fast slot, so a reader transferred from a routing alias cannot be missed. `Kernel::collapse_buffers` moves alias consumers to their root (lines 1515–1553) before deriving `read` from all lists (lines 1575–1586), [src/kernel/mod.rs](../../src/kernel/mod.rs). Thus the observed exception has neither a gate nor a flip-flop reader, nor any other consumer or watch; checking only `F=[]` would not prove that.

Command (ignored reference tests, release):

```sh
FAST_FOLD=cones cargo test --release --test published_values -- --ignored --nocapture --test-threads=1
```

Run on 2026-11-17: **PASS — 2 passed, 0 failed**, 60.18 s of test execution (life_full 50.94 s; life 7.07 s). Both warm starts were at cycle 0. Life inspected 173,521 overlay entries / 15,995 overlay nets; life_full inspected 1,352,363 entries / 117,055 nets. Each fixture had four allowed differences and **zero unexpected differences**. At cycles 1, 7, 100, and 1000, results were:

| Class | life equal | life differing | life_full equal | life_full differing |
| --- | ---: | ---: | ---: | ---: |
| All overlay entries | 694,080 | 4 | 5,409,448 | 4 |
| Folded | 83,272 | 0 | 633,320 | 0 |
| On-demand | 16 | 0 | 24 | 0 |
| Edge-fresh (includes the unused carry sums) | 610,792 | 4 | 4,776,100 | 4 |
| Pad output (overlapping physical tag) | 9,004 | 0 | 59,716 | 0 |
| EBR output | 0 | 0 | 0 | 0 |
| Unused carry sum (overlapping physical tag) | 0 | 4 | 0 | 4 |
| Unread net (overlapping consumer tag) | 20 | 4 | 28 | 4 |
| Missing fast slot | 0 | 0 | 0 | 0 |
| Outside event plan (unavailable, not a known value) | 0 | 0 | 4 | 0 |

Neither fixture exercises EBR outputs, and neither produced event Z in these checks; those zero counts are coverage limits, not proof of agreement. The single life_full entry outside the event plan is checked as unavailable at each checkpoint.

The only differing physical wire per fixture is:

- life: `R29C13:F0_SLICE`, overlay net 169453, three drawn pieces: published 0 versus event X at all four cycles.
- life_full: `R38C56:F0_SLICE`, overlay net 1320467, three drawn pieces: published 0 versus event X at all four cycles.

Both are the F sum output of `$nextpnr_CCU2C_0$CCU2_COMB0`, placed at `X13/Y29/SLICEA.K0` and `X56/Y38/SLICEA.K0`, respectively. In each fixture's `target/reference/<design>/top_routed.json`, that cell has `MODE=CCU2`, `CCU2_INJECT1=NO`, empty `F` and `FCI` connections, and a connected `FCO`. No routed netname's ROUTING string contains that `F0_SLICE` wire. More decisively, exhaustive `Names::resolved` → `Names::overlay_nets` lookup finds **HDL names []** for both overlay nets, including aliases/pins. Name-only hover cannot reach them; location fallback remains prohibited.

For each differing wire and checkpoint the test also evaluates the real compiled carry gate's inputs through `PlanPrefix::carry_details`, using the cached gate/driver mapping and the current kernel values/LUT. The measured values are `carry_output=false`, `inject=false`, `cin=X`, and `propagate=Zero`; life has A/B/C/D = `[One, Zero, One, One]`, life_full has `[Zero, One, One, One]`. The reader audit reports `no_reader=true` for both, after aggregating aliases and watches. This verifies the cause and consumer classification rather than inferring them from the physical wire name.

The code explains the distinction: `Builder::carry` binds **both** sum and carry-out gates ([src/kernel/mod.rs](../../src/kernel/mod.rs), lines 726–745). The event `Kernel::settle_combinational` computes the non-injected sum as `p.xor(cin)` (line 1642): here 0 XOR X is X. Fast generation permits a Boolean carry-in substitution when `!propagates(init, &t)` ([src/kernel/fast.rs](../../src/kernel/fast.rs), `FastKernel::program`, lines 1352–1358); here p is identically 0 and the unconnected carry-in cannot be converted to a Boolean term, so it substitutes 0. The constant sum then reduces to `Term::Lit(0)` (lines 1385–1389), stored in `initial` (lines 1421–1425). This happens before full-settle code generation, so another settle does not restore the event kernel's unknown. The passing pin records this unread-X-to-Boolean class explicitly without finding a folded/on-demand settling failure. No source-name hover can reach the measured exceptions; no helper-specific UI mask or `live.rs` change is needed for these fixtures.

Validation: `cargo fmt --all` (plus direct rustfmt of the new integration test) and `cargo clippy --release --test published_values -- -D warnings` passed. The regression added `tests/published_values.rs` and updated this design note; emulator/GUI source was unchanged and no commit was created.
