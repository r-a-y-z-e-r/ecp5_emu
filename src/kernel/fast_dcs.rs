//! DCSC's generated, per-simulation state. Keep the event implementation as
//! the reference; this is its two-valued wave capture, never a settle gate.
use super::{dcs::State, fast::Term, *};
use std::fmt::Write as _;

pub(super) struct Code {
    pub capture: String,
    pub commit: String,
    pub begin: String,
    pub blocked: Vec<usize>,
}

pub(super) fn generate(
    dcs: &[dcs::Dcs],
    now: Time,
    inputs: &[[Term; 5]],
    outputs: &[usize],
    initial: &mut Vec<u8>,
) -> Result<Code> {
    let mut code = Code {
        capture: String::new(),
        commit: String::new(),
        begin: String::new(),
        blocked: Vec::new(),
    };
    if !dcs.is_empty() {
        code.commit.push_str("    int dcs_changed = 0;\n");
    }
    for (i, dcs) in dcs.iter().enumerate() {
        let unknown = || {
            StartsUnknown(format!(
                "DCSC {i} clock/control or handshake (unknown controls or an uncertain clock edge)"
            ))
        };
        // Unknown constants may be irrelevant to a static table entry. The
        // evaluator rejects only an ambiguous target/output/handshake edge.
        let bit = |v: Logic| v.bit().map_or(2, u8::from);
        if initial[outputs[i]] > 1 {
            return Err(unknown().into());
        }
        // previous clocks, packed controls, state, old/followed source, new
        // source (2 means constant), remaining new edges, same-time guard.
        let mut state = [0; 8];
        state[0] = bit(dcs.previous[0]);
        state[1] = bit(dcs.previous[1]);
        let control = dcs.control.ok_or_else(unknown)?;
        state[2] = bit(control[0]) + 3 * bit(control[1]) + 9 * bit(control[2]);
        match dcs.state {
            State::Follow(n) => state[4] = n as u8,
            State::WaitOld { old, new } => {
                state[3] = 1;
                state[4] = old as u8;
                state[5] = new.map_or(2, |n| n as u8);
            }
            State::WaitNew {
                new,
                remaining,
                after,
            } => {
                state[3] = 2;
                state[5] = new as u8;
                state[6] = remaining;
                state[7] = u8::from(now <= after);
            }
            State::Held => state[3] = 3,
            State::Initial | State::Unknown => return Err(unknown().into()),
        }
        let mode = match dcs.config.mode {
            DcsMode::Pos => 0,
            DcsMode::Neg => 1,
            DcsMode::Clk0Low => 2,
            DcsMode::Clk0High => 3,
            DcsMode::Clk1Low => 4,
            DcsMode::Clk1High => 5,
            DcsMode::Clk0 => 6,
            DcsMode::Clk1 => 7,
            DcsMode::Low => 8,
            DcsMode::High => 9,
        };
        let base = initial.len();
        initial.extend(state);
        code.blocked.push(base + 7);
        let _ = writeln!(code.begin, "  v[{}] = 0;", base + 7);
        let args = inputs[i].map(Term::c).join(", ");
        let _ = writeln!(
            code.capture,
            "    u8 dcs{i} = capture_dcs(&v[{base}], {mode}, {args});"
        );
        let _ = writeln!(
            code.capture,
            "    if (dcs{i} > 1) return fast_fail(v, 7, {i});"
        );
        let q = outputs[i];
        let _ = writeln!(
            code.commit,
            "    dcs_changed |= v[{q}] != dcs{i}; v[{q}] = dcs{i};"
        );
    }
    Ok(code)
}

pub(super) const PRELUDE: &str = r#"
/* DCS state slots: previous clocks, controls, state (follow/wait old/wait
   new/held), old, new, remaining, blocked at this physical timestamp. */
static u8 capture_dcs(u8 *s, unsigned mode, u8 c0, u8 c1, u8 sel0, u8 sel1, u8 bypass) {
  const u8 table[10][4] = {
    {3,0,1,3}, {2,0,1,2}, {2,0,2,2}, {3,0,3,3}, {2,2,1,2},
    {3,3,1,3}, {0,0,0,0}, {1,1,1,1}, {2,2,2,2}, {3,3,3,3}
  };
  c0 = c0 > 1 ? 2 : c0; c1 = c1 > 1 ? 2 : c1;
  sel0 = sel0 > 1 ? 2 : sel0; sel1 = sel1 > 1 ? 2 : sel1;
  bypass = bypass > 1 ? 2 : bypass;
  unsigned control = sel0 + 3 * sel1 + 9 * bypass;
  u8 target = 4;
  for (unsigned b = 0; b < 2; b++) for (unsigned sel = 0; sel < 4; sel++) {
    if ((bypass < 2 && bypass != b) || (sel0 < 2 && sel0 != (sel & 1)) ||
        (sel1 < 2 && sel1 != (sel >> 1))) continue;
    u8 t = b ? (sel == 1 ? 0 : sel == 2 ? 1 : 2) : table[mode][sel];
    if (target != 4 && target != t) return 2;
    target = t;
  }
  u8 clocks[2] = {c0, c1};
  u8 active = mode == 0;
  u8 edges[2];
  for (unsigned n = 0; n < 2; n++) {
    u8 p = s[n], c = clocks[n];
    edges[n] = p == c ? 0 : p == !active && c == active ? 1 :
      (p > 1 || p == !active) && (c > 1 || c == active) ? 2 : 0;
  }
  int changed = s[2] != control;
  s[0] = c0; s[1] = c1; s[2] = control;
  if (mode > 1 || bypass != 0) {
    s[3] = target < 2 ? 0 : 3;
    s[4] = target;
    return target < 2 ? clocks[target] : target - 2;
  }
  if (changed) {
    if (s[3] <= 1) {
      s[3] = target == s[4] ? 0 : 1;
      s[5] = target < 2 ? target : 2;
    } else if (target < 2) {
      s[3] = 2; s[5] = target; s[6] = 2; s[7] = 1;
    } else s[3] = 3;
  }
  if (s[3] == 1 && edges[s[4]]) {
    if (edges[s[4]] == 2) return 2;
    s[3] = s[5] < 2 ? 2 : 3;
    s[6] = 2; s[7] = 1;
  } else if (s[3] == 2 && !s[7] && edges[s[5]]) {
    if (edges[s[5]] == 2) return 2;
    s[7] = 1;
    if (--s[6] == 0) { s[3] = 0; s[4] = s[5]; }
  }
  return s[3] <= 1 ? clocks[s[4]] : active;
}
"#;
