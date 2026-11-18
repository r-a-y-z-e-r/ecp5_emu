#!/usr/bin/env python3
"""make_harness.py program.c harness.c : a 10-thread harness over a generated fast-mode program.

Each thread k runs the pieces of settle_group0_p0_t<k> every iteration; iterations are
separated by a barrier (generation counter published by thread 0; completion by a done
counter or by per-part flags). Modes (argv): alone | private | shared | cycle |
owner | ownercheck | busy <ns>.
"""
from collections import Counter, defaultdict
import re
import sys

src = open(sys.argv[1]).read()
lines = src.split("\n")
threads = int(re.search(r"#define THREADS (\d+)", src).group(1))


def pieces_of(header):
    i = next(k for k, l in enumerate(lines) if l.startswith(header))
    out = []
    for l in lines[i + 1:]:
        if l.startswith("}"):
            break
        m = re.match(r"^\s+(piece\d+)\(v\);", l)
        if m:
            out.append(int(m.group(1)[5:]))
        elif "pool_run" in l:
            break
    return out


parts = [pieces_of(f"static void settle_group0_p0_t{t}(") for t in range(threads)]
shared = pieces_of("static void settle_group0(")
phase1 = pieces_of("static void settle_group0_p1_t0(")
conv = next(k for k, l in enumerate(lines) if l.startswith("static int converge"))
capture = []
for l in lines[conv:]:
    if l.startswith("}"):
        break
    m = re.match(r"^\s+(piece\d+)\(v\);", l)
    if m:
        capture.append(int(m.group(1)[5:]))
n = max(int(x) for x in re.findall(r"^void piece(\d+)\(", src, re.M)) + 1


def piece_bodies():
    bodies = {}
    current = None
    for line in lines:
        m = re.search(r"\bvoid piece(\d+)\(u8 .*\) \{$", line)
        if m:
            current = int(m.group(1))
            bodies[current] = []
        elif current is not None:
            if line.startswith("}"):
                current = None
            elif line.strip():
                bodies[current].append(line)
    return bodies


bodies = piece_bodies()
writers = defaultdict(set)
for t, part in enumerate(parts):
    for p in part:
        for line in bodies[p]:
            # A table-bit line has a local index before its output assignment.
            for slot in re.findall(r"(?:^\s*|;\s*)v\[(\d+)\]\s*=", line):
                writers[int(slot)].add(t)

# The tables have external linkage in multi-unit programs. If a program instead
# keeps a table/helper private, call its original piece in group 2 rather than copy it.
table_decls = re.findall(r"^extern const (?:u8|uint64_t) (?:wide|wide_bits)\[\][^;]*;", src, re.M)
table_names = {re.search(r"\b(wide|wide_bits)\[", d).group(1) for d in table_decls}
known_names = {"v", "n", "unsigned", "i", "u8", "uint32_t", "uint64_t"} | table_names


def can_copy(line):
    # Only copy complete statements, with no dependencies on a piece's scope.
    simple = re.fullmatch(r"\s*[vn]\[\d+\]\s*=\s*[^;{}]+;\s*", line)
    indexed = re.fullmatch(r"\s*\{ unsigned i = [^;]+; v\[\d+\] = [^;]+; }\s*", line)
    without_numbers = re.sub(r"\b(?:0[xX][\da-fA-F]+|\d+)[uUlL]*\b", "", line)
    names = set(re.findall(r"\b[A-Za-z_]\w*\b", without_numbers))
    return (simple or indexed) and names <= known_names


def capture_writes(line):
    return {int(s) for s in re.findall(r"\bv\[(\d+)\]\s*=(?!=)", line)}


def capture_reads(line):
    # Only reads of other destinations can create capture hazards. An indexed
    # table line also reads before its output assignment.
    expression = re.sub(r"\bv\[\d+\](?=\s*=(?!=))", "", line)
    return {int(s) for s in re.findall(r"\bv\[(\d+)\]", expression)} - capture_writes(line)


capture_lines = [(p, i, line) for p in capture for i, line in enumerate(bodies[p])]
all_reads = set().union(*(capture_reads(line) for _, _, line in capture_lines))
# R2: captured registers read by a capture with a different destination.
hazard_registers = set().union(*(capture_writes(line) for _, _, line in capture_lines)) & all_reads
# Keep whole pieces with private state or unsupported statements intact. Keeping
# all temporary pieces here also preserves n[] across the original piece calls.
serial_pieces = {p for p in capture if any(not can_copy(line) or re.search(r"\bn\[", line)
                                         for line in bodies[p])}
group2 = {(p, i) for p, i, line in capture_lines
          if capture_writes(line) & hazard_registers or p in serial_pieces}
while True:
    # Group 2 runs after group 1, so close over its input registers' captures.
    # Group-1 readers of group-2 outputs safely see the values before the edge.
    needed = set().union(*(capture_reads(line) for p, i, line in capture_lines if (p, i) in group2))
    sources = {(p, i) for p, i, line in capture_lines if capture_writes(line) & needed}
    if sources <= group2:
        break
    group2 |= sources
group2_registers = set().union(*(capture_writes(line) for p, i, line in capture_lines if (p, i) in group2))


owned = [[] for _ in range(threads)]
owned_counts = [0] * threads
serial = []
serial_count = 0
t0_reasons = Counter()
for p in capture:
    if p in serial_pieces:
        serial.append(f"  piece{p}(v);")
        serial_count += len(bodies[p])
        continue
    for i, line in enumerate(bodies[p]):
        if (p, i) in group2:
            serial.append(line)
            serial_count += 1
            continue
        t = 0
        m = re.fullmatch(r"\s*v\[(\d+)\]\s*=\s*([^;]+);\s*", line)
        if m and not re.search(r"\bn\[", m.group(2)):
            reads = {int(s) for s in re.findall(r"\bv\[(\d+)\]", m.group(2))} - {int(m.group(1))}
            owners = set().union(*(writers[s] for s in reads))
            if len(owners) == 1:
                t = next(iter(owners))
                reason = "owned by part 0"
            else:
                reason = "no part writer" if not owners else "multiple part writers"
        else:
            reason = "other form / temporary or commit"
        owned[t].append(line)
        owned_counts[t] += 1
        if t == 0:
            t0_reasons[reason] += 1


def arr(name, xs):
    return f"static const int {name}[] = {{{','.join(map(str, xs)) or '-1'}}}; static const int {name}_n = {len(xs)};\n"


h = ["#define _GNU_SOURCE", "#include <stdint.h>", "#include <stdio.h>", "#include <stdlib.h>", "#include <string.h>",
     "#include <pthread.h>", "#include <stdatomic.h>", r"""
#ifdef __APPLE__
#include <mach/mach_time.h>
#include <pthread/qos.h>
static void pin(int t) { (void)t; pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0); }
#else
/* Linux (rserv): a monotonic clock in ns for mach_absolute_time; PIN=c0,c1,...
   pins thread t to CPU c_t. */
#include <sched.h>
#include <time.h>
static uint64_t mach_absolute_time(void) {
  struct timespec ts;
  clock_gettime(CLOCK_MONOTONIC, &ts);
  return (uint64_t)ts.tv_sec * 1000000000u + (uint64_t)ts.tv_nsec;
}
typedef struct { uint32_t numer, denom; } mach_timebase_info_data_t;
static void mach_timebase_info(mach_timebase_info_data_t *t) { t->numer = 1; t->denom = 1; }
static void pin(int t) {
  const char *p = getenv("PIN");
  for (int k = 0; p && *p; k++) {
    int cpu = atoi(p);
    if (k == t) {
      cpu_set_t set;
      CPU_ZERO(&set);
      CPU_SET(cpu, &set);
      pthread_setaffinity_np(pthread_self(), sizeof set, &set);
      return;
    }
    p = strchr(p, ',');
    if (p) p++;
  }
}
#endif""",
     "typedef uint8_t u8;"]
h += [f"void piece{i}(u8 *restrict v);" for i in range(n)]
h += table_decls
for t in range(threads):
    h.append(f"static void cap_t{t}(u8 *restrict v) {{")
    h.extend(owned[t])
    h.append("}")
h.extend(["static void cap_group2(u8 *restrict v) {", *serial, "}"])
h.append("static void (*const caps[])(u8 *restrict) = {" + ",".join(f"cap_t{t}" for t in range(threads)) + "};")
h.append("static void (*const pieces[])(u8 *restrict) = {" + ",".join(f"piece{i}" for i in range(n)) + "};")
for t in range(threads):
    h.append(arr(f"part{t}", parts[t]))
h.append("static const int *const part[] = {" + ",".join(f"part{t}" for t in range(threads)) + "};")
h.append("static const int part_n[] = {" + ",".join(f"part{t}_n" for t in range(threads)) + "};")
h.append(arr("cap", capture))
h.append(arr("shr", shared))
h.append(arr("ph1", phase1))
h.append(f"#define THREADS {threads}")
h.append(r'''
struct line { _Atomic uint32_t v; char pad[124]; } __attribute__((aligned(128)));
static struct line gen, counter, done[64];
static int flags, mode; /* mode: 0 private, 1 shared, 2 cycle, 3 busy, 4 owner */
static uint64_t busy_ticks;
static u8 *arrays[64];
static long iters;
static uint64_t part_ticks[64][16];
static void run(const int *list, int k, u8 *v) { for (int i = 0; i < k; i++) pieces[list[i]](v); }
static void work(int t) {
  if (mode == 3) { uint64_t e = mach_absolute_time() + busy_ticks; while (mach_absolute_time() < e) continue; return; }
  run(part[t], part_n[t], arrays[mode == 0 ? t : 0]);
}
static void *worker(void *arg) {
  int t = (int)(intptr_t)arg;
  pin(t);
  uint32_t seen = 0;
  for (long it = 0; it < iters; it++) {
    if (mode == 4) {
      uint32_t g;
      while ((g = atomic_load_explicit(&gen.v, memory_order_acquire)) == seen) ;
      seen = g;
      caps[t](arrays[0]);
      atomic_store_explicit(&done[t].v, g, memory_order_release);
      while ((g = atomic_load_explicit(&gen.v, memory_order_acquire)) == seen) ;
      seen = g;
      uint64_t t0 = mach_absolute_time();
      work(t);
      part_ticks[t][0] += mach_absolute_time() - t0;
      atomic_store_explicit(&done[t].v, g, memory_order_release);
      continue;
    }
    uint32_t g;
    while ((g = atomic_load_explicit(&gen.v, memory_order_acquire)) == seen) ;
    seen = g;
    uint64_t t0 = mach_absolute_time();
    work(t);
    part_ticks[t][0] += mach_absolute_time() - t0;
    if (flags) atomic_store_explicit(&done[t].v, g, memory_order_release);
    else atomic_fetch_add_explicit(&counter.v, 1, memory_order_release);
  }
  return 0;
}
int main(int argc, char **argv) {
  /* harness MODE FLAGS ITERS REPS [BUSY_NS], or harness ownercheck */
  int check = argc > 1 && !strcmp(argv[1], "ownercheck");
  if (argc < 2 || (!check && argc < 5)) {
    fprintf(stderr, "usage: %s MODE FLAGS ITERS REPS [BUSY_NS], or %s ownercheck\n", argv[0], argv[0]);
    return 1;
  }
  mode = !strcmp(argv[1], "private") ? 0 : !strcmp(argv[1], "shared") ? 1 : !strcmp(argv[1], "cycle") ? 2 : !strcmp(argv[1], "owner") ? 4 : 3;
  flags = check ? 1 : atoi(argv[2]); iters = check ? 100 : atol(argv[3]); int reps = check ? 1 : atoi(argv[4]);
  mach_timebase_info_data_t tb; mach_timebase_info(&tb);
  double ns_tick = (double)tb.numer / tb.denom;
  if (mode == 3 && argc > 5) busy_ticks = (uint64_t)(atof(argv[5]) / ns_tick);
  pin(0);
  for (int t = 0; t < THREADS; t++) {
    arrays[t] = aligned_alloc(128, 1 << 20);
    srand(1); for (int i = 0; i < 1 << 20; i++) arrays[t][i] = rand() & 1;
  }
  if (check) {
    u8 *original = arrays[0], *owner = arrays[1];
    memcpy(owner, original, 1 << 20);
    for (int it = 0; it < 100; it++) {
      run(cap, cap_n, original);
      run(shr, shr_n, original);
      for (int t = 0; t < THREADS; t++) run(part[t], part_n[t], original);
      run(ph1, ph1_n, original);
      for (int t = 0; t < THREADS; t++) caps[t](owner);
      cap_group2(owner);
      run(shr, shr_n, owner);
      for (int t = 0; t < THREADS; t++) run(part[t], part_n[t], owner);
      run(ph1, ph1_n, owner);
      if (memcmp(original, owner, 1 << 20)) {
        for (int s = 0; s < 1 << 20; s++) if (original[s] != owner[s]) {
          fprintf(stderr, "ownercheck FAIL: iteration %d slot %d: original %u owner %u\n", it + 1, s, original[s], owner[s]);
          return 1;
        }
      }
    }
    printf("ownercheck OK: 100 iterations, all %d bytes identical after each iteration\n", 1 << 20);
    return 0;
  }
  if (!strcmp(argv[1], "alone")) {
    for (int t = 0; t < THREADS; t++) {
      double best = 1e30;
      for (int rep = 0; rep < reps; rep++) {
        uint64_t a = mach_absolute_time();
        for (long it = 0; it < iters; it++) run(part[t], part_n[t], arrays[0]);
        double us = (mach_absolute_time() - a) * ns_tick / iters / 1000;
        if (us < best) best = us;
      }
      printf("part %d alone: %.3f us\n", t, best);
    }
    printf("capture alone: "); { double best = 1e30; for (int rep = 0; rep < reps; rep++) { uint64_t a = mach_absolute_time(); for (long it = 0; it < iters; it++) run(cap, cap_n, arrays[0]); double us = (mach_absolute_time() - a) * ns_tick / iters / 1000; if (us < best) best = us; } printf("%.3f us\n", best); }
    printf("shared alone: "); { double best = 1e30; for (int rep = 0; rep < reps; rep++) { uint64_t a = mach_absolute_time(); for (long it = 0; it < iters; it++) run(shr, shr_n, arrays[0]); double us = (mach_absolute_time() - a) * ns_tick / iters / 1000; if (us < best) best = us; } printf("%.3f us\n", best); }
    printf("phase1 alone: "); { double best = 1e30; for (int rep = 0; rep < reps; rep++) { uint64_t a = mach_absolute_time(); for (long it = 0; it < iters; it++) run(ph1, ph1_n, arrays[0]); double us = (mach_absolute_time() - a) * ns_tick / iters / 1000; if (us < best) best = us; } printf("%.3f us\n", best); }
    return 0;
  }
  for (int rep = 0; rep < reps; rep++) {
    memset(part_ticks, 0, sizeof part_ticks);
    atomic_store(&gen.v, 0); atomic_store(&counter.v, 0);
    for (int t = 0; t < THREADS; t++) atomic_store(&done[t].v, 0);
    pthread_t th[64];
    for (int t = 1; t < THREADS; t++) pthread_create(&th[t], 0, worker, (void *)(intptr_t)t);
    uint64_t sync = 0, pre = 0, post = 0, cap_sync = 0, cap_serial = 0, start = mach_absolute_time();
    for (long it = 0; it < iters; it++) {
      if (mode == 4) {
        uint32_t g = (uint32_t)(2 * it) + 1;
        uint64_t a = mach_absolute_time();
        atomic_store_explicit(&gen.v, g, memory_order_release);
        caps[0](arrays[0]);
        for (int t = 1; t < THREADS; t++) while (atomic_load_explicit(&done[t].v, memory_order_acquire) != g) ;
        uint64_t b = mach_absolute_time();
        cap_group2(arrays[0]);
        uint64_t s = mach_absolute_time();
        run(shr, shr_n, arrays[0]);
        uint64_t c = mach_absolute_time();
        atomic_store_explicit(&gen.v, g + 1, memory_order_release);
        uint64_t t0 = mach_absolute_time();
        work(0);
        part_ticks[0][0] += mach_absolute_time() - t0;
        for (int t = 1; t < THREADS; t++) while (atomic_load_explicit(&done[t].v, memory_order_acquire) != g + 1) ;
        uint64_t d = mach_absolute_time();
        run(ph1, ph1_n, arrays[0]);
        uint64_t e = mach_absolute_time();
        cap_sync += b - a; cap_serial += s - b; pre += c - b; sync += d - c; post += e - d;
        continue;
      }
      uint64_t a = mach_absolute_time();
      if (mode == 2) { run(cap, cap_n, arrays[0]); run(shr, shr_n, arrays[0]); }
      uint64_t b = mach_absolute_time();
      uint32_t g = (uint32_t)it + 1;
      atomic_store_explicit(&gen.v, g, memory_order_release);
      uint64_t t0 = mach_absolute_time();
      work(0);
      part_ticks[0][0] += mach_absolute_time() - t0;
      if (flags) { for (int t = 1; t < THREADS; t++) while (atomic_load_explicit(&done[t].v, memory_order_acquire) != g) ; }
      else { while (atomic_load_explicit(&counter.v, memory_order_acquire) != (uint32_t)(THREADS - 1) * g) ; }
      uint64_t c = mach_absolute_time();
      if (mode == 2) run(ph1, ph1_n, arrays[0]);
      uint64_t d = mach_absolute_time();
      pre += b - a; sync += c - b; post += d - c;
    }
    uint64_t total = mach_absolute_time() - start;
    for (int t = 1; t < THREADS; t++) pthread_join(th[t], 0);
    double mx = 0, mean = 0;
    for (int t = 0; t < THREADS; t++) { double p = part_ticks[t][0] * ns_tick / iters / 1000; mean += p / THREADS; if (p > mx) mx = p; }
    if (mode == 4) printf("rep %d: iteration %.3f us | before publish %.3f, publish-to-all-done %.3f, after %.3f | part mean %.3f max %.3f us | capture publish-to-all-done %.3f us | capture group 2 %.3f, shared %.3f us\n",
           rep, total * ns_tick / iters / 1000, pre * ns_tick / iters / 1000, sync * ns_tick / iters / 1000,
           post * ns_tick / iters / 1000, mean, mx, cap_sync * ns_tick / iters / 1000,
           cap_serial * ns_tick / iters / 1000, (pre - cap_serial) * ns_tick / iters / 1000);
    else printf("rep %d: iteration %.3f us | before publish %.3f, publish-to-all-done %.3f, after %.3f | part mean %.3f max %.3f us\n",
           rep, total * ns_tick / iters / 1000, pre * ns_tick / iters / 1000, sync * ns_tick / iters / 1000,
           post * ns_tick / iters / 1000, mean, mx);
    fflush(stdout);
  }
  return 0;
}
''')
open(sys.argv[2], "w").write("\n".join(h))
print(f"threads {threads}, parts {[len(p) for p in parts]} pieces, capture {len(capture)}, shared {len(shared)}, phase1 {len(phase1)}", file=sys.stderr)
print(f"owner capture groups: group 1 {sum(owned_counts)}, group 2 {serial_count}", file=sys.stderr)
print(f"group 2 registers: {len(group2_registers)}" +
      (" (" + ", ".join(map(str, sorted(group2_registers))) + ")" if len(group2_registers) <= 64 else ""), file=sys.stderr)
print("group 1 capture lines: " + ", ".join(f"t{t}={count}" for t, count in enumerate(owned_counts)), file=sys.stderr)
print("group 1 thread 0 reasons: " + (", ".join(f"{reason}: {count}" for reason, count in sorted(t0_reasons.items())) or "none"), file=sys.stderr)
