"""Research experiments against installed Yosys/Icarus models, not emulator code."""
from collections import Counter
from itertools import product
import json
from pathlib import Path
import subprocess


def run(command, cwd):
    result = subprocess.run([str(x) for x in command], cwd=cwd, text=True, capture_output=True)
    if result.returncode:
        raise RuntimeError(f"{Path(command[0]).name}: {result.stderr[-4000:]}")
    return result


def simulate(scratch, name, source, library, iverilog, vvp, carry=False):
    tb = scratch / f"{name}.v"
    tb.write_text(source)
    command = [iverilog, "-g2012", "-DNO_INCLUDES", "-s", "tb", "-I", library,
               "-o", name, library / "common_sim.vh"]
    if carry:
        command.append(library / "ccu2c_sim.vh")
    run(command + [tb.name], scratch)
    result = run([vvp, name], scratch)
    trace = scratch / f"{name}.log"
    trace.write_text(result.stdout)
    return result.stdout.splitlines(), [(f"{name}:testbench", tb), (f"{name}:trace", trace)]


def ff_probes(scratch, library, iverilog, vvp):
    names = ("CEMUX", "CLKMUX", "LSRMUX", "SRMODE", "REGSET", "LSRMODE", "GSR")
    variants = [dict(zip(names, row)) for row in product(
        ("CE", "INV", "0", "1"), ("CLK", "INV"), ("LSR", "INV"),
        ("LSR_OVER_CE", "ASYNC"), ("RESET", "SET"), ("LSR", "PRLD"),
        ("ENABLED", "DISABLED"))]
    instances = []
    for i, params in enumerate(variants):
        values = ",".join(f'.{name}("{value}")' for name, value in params.items())
        instances.append(f"TRELLIS_FF #({values}) f{i}(.CLK(clk),.LSR(lsr),.CE(ce),.DI(d),.M(m),.Q(q[{i}]));")

    # Every data/control change and every clock edge gets its own stable observation.
    events = [("startup_observed_only", (0, 0, 0, 0, 0))]
    for lsr, ce, d, m in product(range(2), repeat=4):
        events += [("controls", (0, lsr, ce, d, m)),
                   ("rising", (1, lsr, ce, d, m)),
                   ("falling", (0, lsr, ce, d, m))]
    # Separate asynchronous assertion, held-reset preload changes and reset release.
    events += [("prepare_preload", (0, 0, 0, 0, 0)),
               ("assert_lsr", (0, 1, 0, 0, 0)),
               ("held_lsr_M_change", (0, 1, 0, 0, 1)),
               ("held_lsr_clock", (1, 1, 0, 0, 1)),
               ("held_lsr_inactive_edge", (0, 1, 0, 0, 1)),
               ("release_lsr", (0, 0, 0, 0, 1)),
               ("held_inverted_lsr_M_change", (0, 0, 0, 0, 0)),
               ("held_inverted_lsr_clock", (1, 0, 0, 0, 0))]
    statements = []
    for i, (_, values) in enumerate(events):
        assignments = ";".join(f"{name}={value}" for name, value in zip(("clk", "lsr", "ce", "d", "m"), values))
        statements.append(f'{assignments};#1;$display("{i} %b",q);')
    source = "module tb;\nreg clk=0,lsr=0,ce=0,d=0,m=0;\n"
    source += f"wire [{len(variants)-1}:0] q;\n" + "\n".join(instances)
    source += "\ninitial begin #1;\n" + "\n".join(statements) + "\n$finish;end endmodule\n"
    lines, sources = simulate(scratch, "ff-models", source, library, iverilog, vvp)
    traces = [line.split() for line in lines if len(line.split()) == 2]
    if len(traces) != len(events):
        raise ValueError("incomplete FF observations")
    states = None
    startup, observations, model_rows = [], [], []
    counters = Counter()
    previous = events[0][1]
    for step_text, trace in traces:
        step = int(step_text)
        actual = list(reversed(trace.lower()))
        if len(actual) != len(variants) or step >= len(events):
            raise ValueError("bad FF trace")
        label, values = events[step]
        clk, lsr, ce, d, m = values
        if step == 0:
            # Time-zero scheduling is itself an observation, not an assumed reset contract.
            states = actual[:]
            startup = [[i, json.dumps(p, sort_keys=True, separators=(",", ":")), actual[i],
                        "observed_software_startup;not_silicon_INIT"] for i, p in enumerate(variants)]
        else:
            for i, p in enumerate(variants):
                edge = previous[0] != clk and clk == int(p["CLKMUX"] == "CLK")
                active_lsr = lsr ^ int(p["LSRMUX"] == "INV")
                asserted = previous[1] != lsr and active_lsr
                if edge or (p["SRMODE"] == "ASYNC" and asserted):
                    if active_lsr:
                        states[i] = str(m if p["LSRMODE"] == "PRLD" else int(p["REGSET"] == "SET"))
                    elif (ce if p["CEMUX"] == "CE" else 1-ce if p["CEMUX"] == "INV" else int(p["CEMUX"])):
                        states[i] = str(d)
                if actual[i] != states[i]:
                    raise ValueError(f"FF {i} mismatch at {step}:{label}: {actual[i]} != {states[i]}")
                counters[i] += 1
        # Adjacent variants differ only in their unused GSR parameter.
        if any(actual[i] != actual[i+1] for i in range(0, len(variants), 2)):
            raise ValueError("GSR parameter unexpectedly changes this model")
        for i, q in enumerate(actual):
            observations.append([step, label, *values, i, q,
                                 "startup_observation" if step == 0 else "matched_public_software_model"])
        previous = values
    for i, p in enumerate(variants):
        model_rows.append([i, *[p[n] for n in names], counters[i], "GSR_not_executed;SD_external;not_hardware"])
    return model_rows, startup, observations, sources


def ccu_probes(scratch, library, iverilog, vvp):
    # All 16 one-hot INIT positions in each half independently, plus complementary,
    # asymmetric and arithmetic patterns. This samples INIT pairs, not all 2^32 pairs.
    pairs = sorted({(1 << i, 0) for i in range(16)} | {(0, 1 << i) for i in range(16)} |
                   {(0, 0), (65535, 65535), (0x6996, 0x6996), (0x9669, 0x6996),
                    (0x1234, 0x8ace), (0xaaaa, 0xcccc), (0x8001, 0x5a5a),
                    (0x000a, 0xffff), (0x0000, 0x000a)})
    variants = [(a, b, i0, i1) for a, b in pairs for i0, i1 in product(("NO", "YES"), repeat=2)]
    instances = []
    for i, (a, b, i0, i1) in enumerate(variants):
        instances.append(f'''CCU2C #(.INIT0(16'h{a:04x}),.INIT1(16'h{b:04x}),.INJECT1_0("{i0}"),.INJECT1_1("{i1}")) c{i}
(.A0(v[0]),.B0(v[1]),.C0(v[2]),.D0(v[3]),.A1(v[4]),.B1(v[5]),.C1(v[6]),.D1(v[7]),.CIN(v[8]),.S0(q[{3*i}]),.S1(q[{3*i+1}]),.COUT(q[{3*i+2}]));''')
    source = f"module tb;reg [8:0] v;wire [{3*len(variants)-1}:0] q;integer j;\n"
    source += "\n".join(instances) + '\ninitial begin for(j=0;j<512;j=j+1)begin v=j;#1;$display("%0d %b",j,q);end $finish;end endmodule\n'
    lines, sources = simulate(scratch, "ccu-models", source, library, iverilog, vvp, carry=True)
    observations = []
    seen = set()
    for line in lines:
        tokens = line.split()
        if len(tokens) != 2: continue
        vector = int(tokens[0]); raw = list(reversed(tokens[1]))
        if vector in seen or len(raw) != len(variants)*3:
            raise ValueError("bad carry trace")
        seen.add(vector)
        for i, (a, b, i0, i1) in enumerate(variants):
            c = vector >> 8
            sums = []
            for init, addr, inject in ((a, vector & 15, i0), (b, (vector >> 4) & 15, i1)):
                p, g = (init >> addr) & 1, (init >> (addr & 3)) & 1
                sums.append(p ^ (c if inject == "NO" else 0))
                c = c if p else (g if inject == "NO" else 0)
            expected = [str(sums[0]), str(sums[1]), str(c)]
            if raw[3*i:3*i+3] != expected:
                raise ValueError(f"CCU2C {i} vector {vector} differs")
            observations.append([i, vector, *sums, c])
    if seen != set(range(512)):
        raise ValueError("missing carry vectors")
    models = [[i, f"0x{a:04x}", f"0x{b:04x}", i0, i1, 512, "matched_public_model;not_hardware"]
              for i, (a, b, i0, i1) in enumerate(variants)]
    return models, observations, sources


def synthesis_probes(scratch, yosys):
    help_text = run([yosys,"-Q","-T","-p","help synth_lattice"],scratch).stdout
    latch_policy = "-latches <info|warn|error>" in help_text
    cases = {
        "init0": "initial q=0; always @(posedge clk) q<=d;",
        "init1": "initial q=1; always @(posedge clk) q<=d;",
        "init1_with_GSR": "GSR g(.GSR(r)); initial q=1; always @(posedge clk) q<=d;",
        "init1_with_SGSR": "SGSR g(.GSR(r),.CLK(clk)); initial q=1; always @(posedge clk) q<=d;",
        "init1_wire_nogsr": "GSR g(.GSR(r)); initial q=1; always @(posedge clk) q<=d;",
        "direct_FF_nogsr": 'GSR g(.GSR(r)); (* nogsr *) TRELLIS_FF #(.GSR("AUTO"),.REGSET("SET")) f(.CLK(clk),.CE(ce),.LSR(1\'b0),.DI(d),.Q(q));',
        "falling_enable_low": "always @(negedge clk) if(!ce) q<=d;",
        "sync_clear_over_ce": "always @(posedge clk) if(r) q<=0; else if(ce) q<=d;",
        "sync_ce_over_clear": "always @(posedge clk) if(ce) begin if(r) q<=0; else q<=d; end",
        "async_set_over_ce": "always @(posedge clk or posedge r) if(r) q<=1; else if(ce) q<=d;",
        "async_preload": "always @(posedge clk or posedge r) if(r) q<=m; else if(ce) q<=d;",
        "async_preload_opt_in": "always @(posedge clk or posedge r) if(r) q<=m; else if(ce) q<=d;",
        "async_preload_noabc9": "always @(posedge clk or posedge r) if(r) q<=m; else if(ce) q<=d;",
        "latch": "always @* if(ce) q=d;",
    }
    if latch_policy:
        cases["latch_permitted"] = "always @* if(ce) q=d;"
        cases["explicit_always_latch"] = "always_latch if(ce) q=d;"
    rows, sources = [], []
    for name, body in cases.items():
        path = scratch / f"synth-{name}.v"
        attr = "(* nogsr *) " if name == "init1_wire_nogsr" else ""
        path.write_text(f"module top(input clk,ce,r,d,m,{attr}output reg q); {body} endmodule\n")
        options = " -asyncprld" if name.startswith("async_preload_") else ""
        if name == "async_preload_noabc9": options += " -noabc9"
        if name == "latch_permitted": options += " -latches info"
        language = " -sv" if name == "explicit_always_latch" else ""
        command = [str(yosys), "-Q", "-T", "-q", "-p", f"read_verilog{language} {path.name}; synth_ecp5{options} -top top -json synth-{name}.json",
                   "-l", f"synth-{name}.log"]
        result = subprocess.run(command, cwd=scratch, text=True, capture_output=True)
        if name == "async_preload":
            if result.returncode == 0 or "cannot be legalized" not in result.stderr or "$_ALDFFE_PPP_" not in result.stderr:
                raise ValueError("asynchronous preload synthesis behavior changed; inspect before changing evidence")
            rows.append([name, "requires_asyncprld_opt_in", "{}", "[]", "[]", "default_flow_rejection;not_silicon_rejection"])
            sources += [(f"synthesis:{name}:source", path), (f"synthesis:{name}:failure_log", scratch / f"synth-{name}.log")]
            continue
        if name == "async_preload_opt_in":
            if result.returncode == 0 or "Failed to get a constant init value" not in result.stderr:
                raise ValueError("ABC9 preload behavior changed; inspect evidence")
            rows.append([name, "ABC9_nonconstant_INIT_rejection", "{}", "[]", "[]", "asyncprld_enabled;works_with_noabc9;not_silicon_rejection"])
            sources += [(f"synthesis:{name}:source", path), (f"synthesis:{name}:failure_log", scratch / f"synth-{name}.log")]
            continue
        if name == "async_preload_noabc9" and result.returncode:
            if "Failed to get a constant init value" not in result.stderr or not latch_policy:
                raise ValueError("preload noabc9 failed for an unclassified reason")
            rows.append([name,"legacy_noabc9_does_not_bypass_ABC9","{}","[]","[]","new_flow_ABC9_nonconstant_INIT_rejection;not_silicon_rejection"])
            sources += [(f"synthesis:{name}:source",path),(f"synthesis:{name}:failure_log",scratch/f"synth-{name}.log")]
            continue
        if name == "latch" and latch_policy:
            if result.returncode == 0 or "Latch inferred for signal" not in result.stderr:
                raise ValueError("default latch policy changed; inspect evidence")
            rows.append([name,"implicit_latch_rejected_by_default","{}","[]","[]","use_latches_info_or_explicit_always_latch;not_silicon_rejection"])
            sources += [(f"synthesis:{name}:source",path),(f"synthesis:{name}:failure_log",scratch/f"synth-{name}.log")]
            continue
        if result.returncode:
            raise RuntimeError(result.stderr[-4000:])
        netlist = scratch / f"synth-{name}.json"
        cells = json.loads(netlist.read_text())["modules"]["top"]["cells"]
        types = Counter(c["type"] for c in cells.values())
        ff = [c for c in cells.values() if c["type"] == "TRELLIS_FF"]
        if len(ff) != (0 if name in ("latch","latch_permitted","explicit_always_latch") else 1):
            raise ValueError(f"unexpected FF count: {name}")
        params = [c["parameters"] for c in ff]
        expected = {
            "init0": {"REGSET": "RESET"}, "init1": {"REGSET": "SET"},
            "init1_with_GSR": {"REGSET": "SET", "GSR": "ENABLED"},
            "init1_with_SGSR": {"REGSET": "SET", "GSR": "ENABLED"},
            "init1_wire_nogsr": {"REGSET": "SET", "GSR": "ENABLED"},
            "direct_FF_nogsr": {"REGSET": "SET", "GSR": "DISABLED"},
            "falling_enable_low": {"CLKMUX": "INV", "CEMUX": "INV"},
            "sync_clear_over_ce": {"SRMODE": "LSR_OVER_CE", "CEMUX": "CE"},
            "async_set_over_ce": {"SRMODE": "ASYNC", "REGSET": "SET"},
            "async_preload_noabc9": {"SRMODE": "ASYNC", "LSRMODE": "PRLD", "GSR": "DISABLED"},
        }.get(name, {})
        if any(params[0].get(k) != v for k, v in expected.items()):
            raise ValueError(f"unexpected mapping: {name}: {params}")
        if name in ("sync_ce_over_clear", "latch", "latch_permitted", "explicit_always_latch"):
            # Check the extra LUT's purpose from its actual netlist connections.
            module = json.loads(netlist.read_text())["modules"]["top"]
            port_bits = {p: v["bits"][0] for p,v in module["ports"].items()}
            lut = next(c for c in cells.values() if c["type"] == "LUT4")
            if types["LUT4"] != 1: raise ValueError("unexpected fabric logic")
            if name != "sync_ce_over_clear" and lut["connections"]["Z"] != [port_bits["q"]]: raise ValueError("latch feedback missing")
            if name == "sync_ce_over_clear" and lut["connections"]["Z"] != ff[0]["connections"]["LSR"]: raise ValueError("reset gate missing")
            for r, ce, d, q in product(range(2), repeat=4):
                values = {port_bits["r"]:r,port_bits["ce"]:ce,port_bits["d"]:d,port_bits["q"]:q,"0":0,"1":1}
                addr = sum(values[lut["connections"][pin][0]] << i for i,pin in enumerate("ABCD"))
                actual = int(lut["parameters"]["INIT"],2) >> addr & 1
                if actual != (r & ce if name == "sync_ce_over_clear" else d if ce else q):
                    raise ValueError("fabric reset/feedback function changed")
        connections = [{"type":c["type"],"parameters":c["parameters"],"connections":c["connections"]} for c in cells.values()]
        rows.append([name, "synthesized", json.dumps(dict(sorted(types.items())), sort_keys=True, separators=(",", ":")),
                     json.dumps(params, sort_keys=True, separators=(",", ":")), json.dumps(connections,sort_keys=True,separators=(",", ":")),
                     "Yosys_structure_only;P_and_R_not_run"])
        sources += [(f"synthesis:{name}:source", path), (f"synthesis:{name}:netlist", netlist)]
    return rows, sources


def probes(scratch, library, yosys, iverilog, vvp):
    scratch = scratch.resolve(); scratch.mkdir(parents=True, exist_ok=True)
    ff, startup, ff_obs, sources = ff_probes(scratch, library, iverilog, vvp)
    ccu, ccu_obs, extra = ccu_probes(scratch, library, iverilog, vvp); sources += extra
    synth, extra = synthesis_probes(scratch, yosys); sources += extra
    return ff, startup, ff_obs, ccu, ccu_obs, synth, sources
