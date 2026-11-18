"""Summarize all 15 run_1 raw captures; no hardware access. JSON to stdout."""
import argparse
import json
from pathlib import Path
import sys
sys.dont_write_bytecode = True
from analyse import parse, STEP, PIN_HZ


def read(directory):
    reports = []
    for profile in 'ABCDE':
        for load in range(1, 4):
            path = directory/f'{profile}.load-{load:03d}.bin'
            records = parse(path.read_bytes())
            unique = {}
            for r in records:
                pair = (r['lock'], r['op'])
                assert r['pin'] not in unique or unique[r['pin']] == pair
                unique[r['pin']] = pair
            assert all(t in unique for t in range(0, 201*STEP, STEP)), path.name
            times = sorted(unique)
            differences = [(unique[b][1]-unique[a][1]) & 0xffffffff
                           for a, b in zip(times, times[1:]) if b-a == STEP and a >= STEP]
            high = [t for t in times if unique[t][0]]
            reports.append(dict(profile=profile, load=load, bytes=path.stat().st_size,
                                records=len(records), startup_samples=201,
                                first_high_lock_ms=min(high)/PIN_HZ*1000,
                                low_lock_ms=[t/PIN_HZ*1000 for t in times if not unique[t][0]],
                                steady_10ms_deltas=sorted(set(differences)),
                                steady_mean_mhz=PIN_HZ*sum(differences)/len(differences)/STEP/1e6))
    return reports


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('run', type=Path)
    print(json.dumps(read(p.parse_args().run), indent=2))
