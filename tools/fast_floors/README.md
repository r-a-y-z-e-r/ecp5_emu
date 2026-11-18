# Fast-mode floors

A harness over a program fast mode generated, for measuring where a threaded cycle's time goes
without the rest of the emulator (docs/FAST_MODE.md, "The full-chip Life: life_full").

Make a program, then build and run the harness:

```bash
mkdir -p out/floors
TMPDIR=$PWD/out/floors/ FAST_TUNE=0 FAST_CHOOSE=0 FAST_THREADS=10 target/release/ecp5_emu emulate \
  target/reference/life_full/top.config --names target/reference/life_full/top_routed.json \
  --fast --startup configured --scope observed --clock net:clk_25mhz --period-ns 40 --cycles 10 --probe net:led
tools/fast_floors/build.sh out/floors/ecp5_emu_fast/<hash>.c out/floors/harness
out/floors/harness alone 0 5000 7     # each part, the captures, the shared lines, phase 1, alone
out/floors/harness private 1 20000 7  # every part at once, each on its own copy of the slots
out/floors/harness shared 1 20000 7   # every part at once on one array, no captures
out/floors/harness cycle 1 20000 7    # captures + shared lines, the parts, phase 1: a whole cycle
out/floors/harness ownercheck        # original vs owner captures, 100 cycles, single-threaded
out/floors/harness owner 1 20000 7    # group 1 captures, meeting, group 2 + shared, parts, phase 1
out/floors/harness busy 1 20000 7 1000 # the meeting alone, around 1000 ns of spinning a part
```

The second argument picks the meeting: 0 one counter every thread adds to, 1 a line a part.
`owner` always uses a line a part for both meetings. It publishes a generation for group 1
captures, each thread runs its captures, then the caller waits for them all and runs group 2
captures in their original order, followed by the shared lines. It publishes the next generation
for the parts, waits again, and runs phase 1. Its columns are the same as `cycle`, plus
`capture publish-to-all-done` (group 1), `capture group 2` and `shared`. `before publish` is
group 2 plus the shared lines; `publish-to-all-done` is the parts' step. All times are averages
per iteration.

R2 is the set of captured registers read on the RHS of a capture with a different destination.
Group 2 starts with the captures of R2, then repeatedly adds the captures of any other registers
its lines read, until no more are added. Self-references do not count; references to other slots
in constant-dead branches still do. Group 1 contains every other capture. Group-1 captures may
read group-2 registers: they see the old values before group 2 writes them. Group-2 captures
cannot read a register written by group 1, which has already run.

A group-1 capture `v[Q] = expression;` goes to the part that writes the slots the expression
reads (excluding Q). Reads with no part writer do not pick an owner; no writer at all, or writers
in different parts, leaves the capture on thread 0. Other forms also stay on thread 0. The build
prints both group sizes, the group-1
line counts per owner and the reasons for thread 0's group-1 lines. It also prints the group-2
register count, listing their slots when there are at most 64.

The generated capture functions can read the program's exported `wide` and `wide_bits` tables.
Pieces with temporaries, private helpers or unsupported statements stay intact in group 2,
as calls to the original pieces, preserving their order and private state.

`ownercheck` starts with two identical copies of the usual seeded array. For 100 iterations it
runs the original captures on one and all group-1 captures in thread order followed by group 2
on the other, each followed by shared lines, all parts and phase 1. It compares the whole arrays
after every iteration, printing OK or the first differing slot and returning nonzero on a
difference. This checks the capture reordering in one thread; it does not check concurrent
capture races.

`false_sharing.py PROGRAM.c` counts, per part, the cache lines it reads that another part writes
in the same phase. Measure on a quiet machine (load average under ~6): the threaded modes are
meaningless under load.

## Register locality

`locality.py PROGRAM.c` prints, for each phase-0 part, how many registers its lines read, the
cache lines they are on, and how many of those registers and lines belong to another thread.
A register belongs to the thread that computes its next value, as owner capture uses. The
first line counts register cache lines holding registers of several threads.

```bash
tools/fast_floors/locality.py out/floors/ecp5_emu_fast/<hash>.c
```

## Linux measurements

The harness builds on Linux (rserv) as well as macOS. On Linux it uses a monotonic clock in
place of `mach_absolute_time`. `PIN=c0,c1,...` pins thread t to CPU c_t on Linux; macOS ignores
it and sets the thread's QoS instead. For an 8-thread program:

```bash
PIN=0,1,2,3,4,5,6,7 out/floors/harness owner 1 20000 3
```

Run the build, generation and measurements from the repository root on an x86-64
Linux machine (the recorded runs used the maintainers' build server, rserv).
Install Rust, Python 3, clang/clang++, and the Trellis source, database, and native
library. Set `TRELLIS_ROOT`, `TRELLIS_PREFIX`, and `ECP5_DB` to those installations.
Build the native image reader and set `ECP5_IMAGE_READER` as shown below. Ensure
`target/reference/life_full/top.config` and `top_routed.json` are present, copying
them from the fixture build if needed. Pick CPUs on the intended CCDs for both
`taskset` and `PIN`:

```bash
set -e
export ECP5_IMAGE_READER="$PWD/target/trellis_image_reader"
python3 tools/build_image_reader.py
cargo build --release --bin ecp5_emu
mkdir -p out/floors
run_dir=$(mktemp -d "$PWD/out/floors/rserv.XXXXXX")
TMPDIR="$run_dir/" FAST_TUNE=0 FAST_CHOOSE=0 FAST_THREADS=8 taskset -c 0-7 \
  target/release/ecp5_emu emulate target/reference/life_full/top.config \
  --names target/reference/life_full/top_routed.json --fast --startup configured \
  --scope observed --clock net:clk_25mhz --period-ns 40 --cycles 10 --probe net:led
for program in "$run_dir"/ecp5_emu_fast/*.c; do
  tools/fast_floors/build.sh "$program" out/floors/harness
  python3 tools/fast_floors/false_sharing.py "$program"
  python3 tools/fast_floors/locality.py "$program"
  out/floors/harness ownercheck
  for mode in alone private shared cycle owner busy; do
    PIN=0,1,2,3,4,5,6,7 taskset -c 0-7 out/floors/harness "$mode" 1 20000 3 1000
  done
done
```
