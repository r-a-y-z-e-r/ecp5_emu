"""Check the HI08 records against the DELAYF control model.
Usage: python3 hi08_model.py pass.txt 10 120 [pass_b.txt 0 127 ...]

Each record is 2048 samples of {loadn, move, direction, cflag1, cflag0}, one
per 25 MHz cycle; the controls change only between samples. Model:
- the 7-bit delay counter starts at 0 after configuration;
- LOADN low loads DEL_VALUE at once and holds it (MOVE does nothing);
- at a falling edge of MOVE (LOADN high) the counter steps by DIRECTION
  (0 up, 1 down), saturating at 127 and 0;
- CFLAG = DIRECTION ? counter == 0 : counter == 127, at once.
DIRECTION changing while MOVE is high is outside the documented protocol;
the board's three such cases fit no simple rule, so from the first such
change to the next LOADN the record is not compared (reported)."""
import sys


def check(path, values):
    x = open(path).read().split()[0]
    bits = bin(int(x[2:], 16))[2:].zfill(10240)
    s = [[int(c) for c in bits[5 * i:5 * i + 5]] for i in range(2048)]
    count = [0, 0]
    bad, skipped, compared, skip = [], 0, 0, False
    for i in range(1, 2048):
        loadn, move, direction = s[i][:3]
        prev_move, prev_dir = s[i - 1][1], s[i - 1][2]
        if move and prev_move and direction != prev_dir:
            skip = True
        if not loadn:
            count = list(values)
            skip = False
        elif prev_move and not move:
            for k in (0, 1):
                step = -1 if direction else 1
                count[k] = min(127, max(0, count[k] + step))
        flags = [int(count[k] == (0 if direction else 127)) for k in (0, 1)]
        observed = [s[i][4], s[i][3]]
        if skip:
            skipped += 1
            continue
        compared += 1
        if flags != observed:
            bad.append(i)
    print(f"{path}: {compared} samples compared, {len(bad)} differ {bad[:8]}; "
          f"{skipped} samples skipped after DIRECTION changed while MOVE was high")
    return not bad


args = sys.argv[1:]
ok = all(check(args[i], (int(args[i + 1]), int(args[i + 2]))) for i in range(0, len(args), 3))
print("model fits" if ok else "NO FIT")
