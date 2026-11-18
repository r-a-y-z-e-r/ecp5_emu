//! A design over the die's fabric (src/fabric_overlay.rs), as the live
//! window shows it: the arcs the plan routes are drawn in the fabric, every
//! piece in use carries a wire's value and lies a finite way from its net's
//! driver, every net starts at a site's pin or at a constant a CIB ties it
//! to, the clock runs from its pad through the clock tree, the values
//! arrive while the design runs, and every wire along a net shows the same
//! value at every edge.
mod common;
use common::reference;
use ecp5_emu::{
    fabric_overlay::Kind,
    live::{self, Command, Design, Shared},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn check(design: &str, root: PathBuf) {
    let shared = Arc::new(Mutex::new(Shared::default()));
    let commands = live::spawn(
        Design {
            config: root.join("top.config"),
            names: Some(root.join("top_routed.json")),
            run: false,
        },
        Arc::clone(&shared),
        || {},
    );
    let start = Instant::now();
    let s = loop {
        {
            let s = shared.lock().unwrap();
            assert!(s.error.is_none(), "{}", s.error.as_ref().unwrap());
            if let Some(ready) = &s.ready {
                break Arc::clone(ready);
            }
        }
        assert!(start.elapsed() < Duration::from_secs(600), "not ready");
        std::thread::sleep(Duration::from_millis(50));
    };
    println!(
        "{design}: ready after {:.1} s",
        start.elapsed().as_secs_f32()
    );
    let o = &s.overlay;
    let count = |k: Kind| o.pieces.iter().filter(|p| p.kind == k).count();
    println!(
        "  {} routes, {} drawn as arcs of the fabric ({} of them left out there); {} wires, {} more through fixed arcs; {} pieces (tracks {}, local {}, jumpers {}, pins {}, links {}, DSP {}, inside slices {}), {} dots, {} LUTs and flip-flops; {} pieces with no driver drawn; longest net {:.2}",
        o.routes,
        o.drawn,
        o.hidden,
        o.values.len(),
        o.carried,
        o.pieces.len(),
        count(Kind::Track),
        count(Kind::Local),
        count(Kind::Jumper),
        count(Kind::Pin),
        count(Kind::Link),
        count(Kind::Block),
        count(Kind::Internal),
        o.dots.len(),
        o.sites.len(),
        o.undriven,
        o.longest,
    );
    println!(
        "  {} other sites in use: {}",
        o.boxes.len(),
        o.boxes
            .iter()
            .take(8)
            .map(|b| b.label.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    assert!(!o.boxes.is_empty(), "the pads at least are in use");
    // The CIBs pass the J wires on to the pads, EBRs and DSPs: a CIB in use
    // carries data, not only constants.
    let cibs: Vec<&ecp5_emu::fabric_overlay::UsedBox> =
        o.boxes.iter().filter(|b| b.label == "CIB").collect();
    let busy = cibs.iter().filter(|b| b.values.len() > 1).count();
    println!(
        "  {} CIBs in use, {busy} of them passing more than one wire",
        cibs.len()
    );
    assert!(busy > 0, "no CIB passes data on");
    let route = o.pieces.iter().filter(|p| p.route).count();
    println!(
        "  {route} pieces on the way from a driver to what it feeds, {} past it; {} pieces of nets that feed nothing in use",
        o.pieces.len() - route,
        o.unfed
    );
    let mut missing: Vec<_> = o.not_drawn.iter().collect();
    missing.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
    for (why, n) in missing.iter().take(15) {
        println!("  not drawn {n:6}  {why}");
    }
    let mut hidden: Vec<_> = o.hidden_by.iter().collect();
    hidden.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
    for (why, n) in hidden.iter().take(15) {
        println!("  left out by the fabric {n:6}  {why}");
    }
    // The busiest tiles, by pieces in use.
    let mut busy: Vec<(usize, u16, u16)> = (0..s.fabric.rows)
        .flat_map(|r| (0..s.fabric.cols).map(move |c| (r, c)))
        .map(|(r, c)| {
            (
                o.pieces_in(r as usize * s.fabric.cols as usize + c as usize)
                    .len(),
                r,
                c,
            )
        })
        .collect();
    busy.sort_by_key(|b| std::cmp::Reverse(b.0));
    println!(
        "  busiest tiles: {}",
        busy.iter()
            .take(6)
            .map(|(n, r, c)| format!("R{r}C{c} ({n})"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    // The board clock's net: from its pad through the clock tree to the
    // clock inputs of the slices.
    let clock = s
        .net_names
        .iter()
        .filter(|(_, n)| {
            n.iter()
                .any(|n| n == "clk_25mhz" || n.starts_with("clk_25mhz ("))
        })
        .map(|(net, _)| *net)
        .max_by_key(|&net| o.pieces_of(net).len())
        .expect("the clock's net has its HDL name");
    let tile = |p: &[f32; 2]| {
        (
            s.fabric.row_at(p[1]),
            (p[0] / ecp5_emu::fabric::TILE_W) as u16,
        )
    };
    let pieces = o.pieces_of(clock);
    let tree = pieces
        .iter()
        .filter(|&&k| o.pieces[k as usize].kind == Kind::Clock && o.pieces[k as usize].route)
        .count();
    let from: Vec<(u16, u16)> = o.drivers_of(clock).iter().map(tile).collect();
    println!(
        "  clk_25mhz: {} pieces ({} on its way, {tree} of the clock tree), from {from:?} to {} site pins",
        pieces.len(),
        pieces.iter().filter(|&&k| o.pieces[k as usize].route).count(),
        o.fed_of(clock).len()
    );
    assert!(from.contains(&(47, 0)), "the clock starts at its pad, G2");
    assert!(tree > 0 && !o.fed_of(clock).is_empty());
    // Every net starts at a site's pin, a net a CIB ties to a constant at
    // its pin at the CIB: by kind of wire and tile types, the starts that
    // do not.
    let mut loose: std::collections::BTreeMap<String, usize> = Default::default();
    for &(net, d) in &o.loose {
        let (r, c) = tile(&d);
        let wires: std::collections::BTreeSet<String> = o
            .pieces_of(net)
            .iter()
            .map(|&k| &o.pieces[k as usize])
            .filter(|p| {
                [p.a, p.b]
                    .iter()
                    .any(|q| (q[0] - d[0]).abs() < 1e-3 && (q[1] - d[1]).abs() < 1e-3)
            })
            .map(|p| {
                let w = &o.values[p.value as usize];
                let family: String = w.name.chars().filter(|c| !c.is_ascii_digit()).collect();
                format!("{family} {:?}", p.kind)
            })
            .collect();
        let names = s
            .net_names
            .get(&net)
            .and_then(|n| n.first())
            .cloned()
            .unwrap_or_default();
        *loose
            .entry(format!(
                "{} in {} (e.g. R{r}C{c} {names})",
                wires.into_iter().collect::<Vec<_>>().join(" + "),
                s.fabric.types_at(r, c).join("+")
            ))
            .or_default() += 1;
    }
    let tied: Vec<(u32, bool)> = o.tied_nets().collect();
    println!(
        "  {} nets tied to a constant by a CIB ({} to 1), starting at {} points",
        tied.len(),
        tied.iter().filter(|t| t.1).count(),
        tied.iter().map(|t| o.drivers_of(t.0).len()).sum::<usize>()
    );
    println!("  {} nets start at no site's pin", o.loose.len());
    let mut loose: Vec<_> = loose.into_iter().collect();
    loose.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (what, n) in loose.iter().take(20) {
        println!("    {n:5}  {what}");
    }
    assert!(loose.is_empty(), "nets that start at no site's pin");
    // A LUT or flip-flop is lit only where a way reaches it, and a way
    // reaches exactly those the plan uses: every lit one is in use, and
    // every one in use is lit (the connections inside the slices reach
    // those whose only use is there: a carry, a sum into its flip-flop).
    let mut idle = Vec::new();
    let mut dark = Vec::new();
    for r in 0..s.fabric.rows {
        for c in 0..s.fabric.cols {
            let p = r as usize * s.fabric.cols as usize + c as usize;
            let plan = s.geometry.logic.iter().find(|t| (t.row, t.col) == (r, c));
            if let Some(t) = plan {
                for n in 0..8 {
                    for (mask, label) in [
                        (t.luts, format!("LUT{n}")),
                        (t.flip_flops, format!("FF{n}")),
                    ] {
                        if mask >> n & 1 == 1 && !o.sites_in(p).iter().any(|x| x.label == label) {
                            dark.push(format!("R{r}C{c} {label}"));
                        }
                    }
                }
            }
            for site in o.sites_in(p) {
                let (mask, n) = match (
                    site.label.strip_prefix("LUT"),
                    site.label.strip_prefix("FF"),
                ) {
                    (Some(n), _) => (plan.map_or(0, |t| t.luts), n),
                    (_, Some(n)) => (plan.map_or(0, |t| t.flip_flops), n),
                    _ => continue,
                };
                let n: u32 = n.parse().unwrap();
                if mask >> n & 1 == 0 {
                    idle.push(format!("R{r}C{c} {}", site.label));
                }
            }
        }
    }
    let lit = o.sites.len();
    let planned: u32 = s
        .geometry
        .logic
        .iter()
        .map(|t| t.luts.count_ones() + t.flip_flops.count_ones())
        .sum();
    println!(
        "  {lit} LUTs and flip-flops lit of {planned} in use; lit but idle: {idle:?}; in use but dark: {dark:?}"
    );
    assert!(
        idle.is_empty(),
        "a way runs to a LUT or flip-flop the plan does not use"
    );
    assert!(
        dark.is_empty(),
        "a LUT or flip-flop in use that no way reaches"
    );
    // Every one lit has an output whose value it shows and flashes with: a
    // LUT its F, or in CCU2 mode the carry of its half where only that is
    // taken, or as RAMW what it passes on; a flip-flop its Q.
    let silent: Vec<String> = o
        .sites
        .iter()
        .filter(|x| x.value == u32::MAX && x.also.is_empty())
        .map(|x| {
            format!(
                "R{}C{} {}",
                s.fabric.row_at(x.min[1]),
                (x.min[0] / ecp5_emu::fabric::TILE_W) as u16,
                x.label
            )
        })
        .collect();
    println!("  lit with no output to show: {silent:?}");
    assert!(
        silent.is_empty(),
        "a LUT or flip-flop lit with no value to show"
    );
    assert!(o.pieces.iter().all(|p| (p.value as usize) < o.values.len()));
    assert!(o
        .pieces
        .iter()
        .all(|p| p.d0.is_finite() && p.d1.is_finite()));
    assert!(o.dots.iter().all(|d| d.d.is_finite()));
    // Every piece lies in the tile it is listed in.
    for r in 0..s.fabric.rows {
        for c in 0..s.fabric.cols {
            let p = r as usize * s.fabric.cols as usize + c as usize;
            let (x0, x1) = (
                c as f32 * ecp5_emu::fabric::TILE_W,
                (c + 1) as f32 * ecp5_emu::fabric::TILE_W,
            );
            let (y0, y1) = (
                s.fabric.row_top[r as usize],
                s.fabric.row_top[r as usize + 1],
            );
            for piece in o.pieces_in(p) {
                for q in [piece.a, piece.b] {
                    assert!(
                        q[0] >= x0 - 1e-3
                            && q[0] <= x1 + 1e-3
                            && q[1] >= y0 - 1e-3
                            && q[1] <= y1 + 1e-3,
                        "piece {piece:?} outside R{r}C{c}"
                    );
                }
            }
        }
    }
    assert!(
        o.drawn * 10 >= o.routes * 9,
        "fewer than 90 % of the routes drawn"
    );
    // Values arrive, and some wires are at 1 and some at 0.
    commands.send(Command::Step(1000)).unwrap();
    let start = Instant::now();
    loop {
        {
            let s = shared.lock().unwrap();
            assert!(s.error.is_none(), "{}", s.error.as_ref().unwrap());
            if s.cycles >= 1000 && s.lit.len() == o.values.len() {
                let ones = o
                    .pieces
                    .iter()
                    .filter(|p| s.lit[p.value as usize] == 1)
                    .count();
                let zeros = o
                    .pieces
                    .iter()
                    .filter(|p| s.lit[p.value as usize] == 0)
                    .count();
                println!(
                    "  after {} cycles: {ones} pieces at 1, {zeros} at 0, {} unknown",
                    s.cycles,
                    o.pieces.len() - ones - zeros
                );
                assert!(ones > 0 && zeros > 0);
                break;
            }
        }
        assert!(start.elapsed() < Duration::from_secs(120), "no values");
        std::thread::sleep(Duration::from_millis(50));
    }
    one_value_along_each_net(&shared, &commands, &s);
}

/// Routing does not invert: once the logic settles, every wire of a net
/// that the window draws on its way carries the same value. Runs the design
/// at a rate that publishes after every edge and looks at each state it
/// publishes; two nets drawn as one, or a wire given another's value, shows
/// as a net with both values at once.
fn one_value_along_each_net(
    shared: &Mutex<Shared>,
    commands: &std::sync::mpsc::Sender<Command>,
    s: &live::Static,
) {
    let o = &s.overlay;
    // The wires on the way of each net, by value.
    let mut along: std::collections::HashMap<u32, Vec<u32>> = Default::default();
    for p in o.pieces.iter().filter(|p| p.route) {
        along
            .entry(o.nets[p.value as usize])
            .or_default()
            .push(p.value);
    }
    for values in along.values_mut() {
        values.sort_unstable();
        values.dedup();
    }
    along.retain(|_, v| v.len() > 1);
    commands.send(Command::Rate(Some(1000.0))).unwrap();
    commands.send(Command::Run).unwrap();
    let start = Instant::now();
    let (mut states, mut first_cycle, mut last_cycle) = (0, None, 0);
    let mut split: std::collections::BTreeMap<u32, (usize, String)> = Default::default();
    // The LUTs and flip-flops lit whose value has been 0 or 1 at least once.
    let mut known = vec![false; o.sites.len()];
    while start.elapsed() < Duration::from_millis(1500) {
        std::thread::sleep(Duration::from_millis(2));
        let sh = shared.lock().unwrap();
        assert!(sh.error.is_none(), "{}", sh.error.as_ref().unwrap());
        if sh.lit.len() != o.values.len() {
            continue;
        }
        states += 1;
        for (k, site) in o.sites.iter().enumerate() {
            known[k] |= std::iter::once(&site.value)
                .chain(&site.also)
                .any(|&w| sh.lit.get(w as usize).is_some_and(|&v| v < 2));
        }
        first_cycle.get_or_insert(sh.cycles);
        last_cycle = sh.cycles;
        for (&net, values) in &along {
            let seen: std::collections::BTreeSet<u8> = values
                .iter()
                .map(|&v| sh.lit[v as usize])
                .filter(|&x| x < 2)
                .collect();
            if seen.len() > 1 {
                let e = split.entry(net).or_insert_with(|| {
                    let wires: Vec<String> = values
                        .iter()
                        .take(6)
                        .map(|&v| {
                            let w = &o.values[v as usize];
                            format!(
                                "{}@R{}C{}={}",
                                w.name, w.location.row, w.location.col, sh.lit[v as usize]
                            )
                        })
                        .collect();
                    (0, wires.join(", "))
                });
                e.0 += 1;
            }
        }
    }
    commands.send(Command::Pause).unwrap();
    println!(
        "  one value along each net: {} nets of several wires, {states} states from cycle {} to {last_cycle}; {} nets with two values at once",
        along.len(),
        first_cycle.unwrap_or(0),
        split.len()
    );
    for (net, (n, wires)) in split.iter().take(10) {
        let names = s
            .net_names
            .get(net)
            .and_then(|n| n.first())
            .cloned()
            .unwrap_or_default();
        println!("    net {net} {names} in {n} states: {wires}");
    }
    assert!(states > 50, "too few states to compare");
    assert!(split.is_empty(), "a net shows two values at once");
    // Every LUT and flip-flop lit shows a value the design gives it, so a
    // change of it can flash.
    let never: Vec<String> = o
        .sites
        .iter()
        .zip(&known)
        .filter(|(_, k)| !**k)
        .map(|(x, _)| {
            format!(
                "R{}C{} {}",
                s.fabric.row_at(x.min[1]),
                (x.min[0] / ecp5_emu::fabric::TILE_W) as u16,
                x.label
            )
        })
        .collect();
    println!(
        "  LUTs and flip-flops never 0 or 1: {} of {}{}",
        never.len(),
        o.sites.len(),
        if never.is_empty() {
            String::new()
        } else {
            format!(": {:?}", &never[..never.len().min(20)])
        }
    );
    assert!(
        never.is_empty(),
        "a LUT or flip-flop lit whose value is never known"
    );
}

#[test]
fn uart_hello_over_the_fabric() {
    check("uart_hello", reference::prepared("uart_hello"));
}

#[test]
#[ignore = "PicoRV32 takes a while: cargo test --release --test fabric_overlay -- --ignored"]
fn picorv32_over_the_fabric() {
    check("picorv32_console", reference::design("picorv32_console"));
}

#[test]
fn blinky_cycle_over_the_fabric() {
    check("blinky_cycle", reference::prepared("blinky_cycle"));
}
