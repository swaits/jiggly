# /// script
# requires-python = ">=3.10"
# dependencies = ["numpy"]
# ///
"""
Monte Carlo tuner for the four lifecycle constants in src/main.rs:

    RUN_DURATION    full cycle length, in minutes
    YELLOW_AT       remaining-minute threshold where breathing-yellow begins
    RED_AT          remaining-minute threshold where breathing-red begins
    FAST_RED_AT     remaining-minute threshold where the fast-pulse blink begins

Sweeps a 4-D grid (with the constraint YELLOW_AT > RED_AT > FAST_RED_AT > 0)
across 50 000 simulated workdays and picks the combination that lands the
screen-sleep in the lunch hour as often as possible.

The model

  - Day starts at Triangular(8:00, mode 8:30, 9:30) and ends at
    Triangular(16:00, mode 17:30, 19:00). Lunch is 12:00–13:00 (fixed).
  - Free RESET at start (boot) and at 13:00 (re-login after lunch).
  - User-at-desk minute-by-minute, sees the LED, and may tap RESET to
    extend the cycle:
        yellow         1.5 % / min
        red            4.0 % / min
        fast-red       6.0 % / min
        warning10/5    one-shot bumps the minute the spiral animation fires
  - A composite score rewards lunch-hour expiration (especially the
    12:15–12:45 sweet spot) and penalizes daytime failures.

Usage

    uv run scripts/tune_runtime.py             # default 50 000 days
    uv run scripts/tune_runtime.py --n 100000  # finer Monte Carlo

Adjust the per-phase press probabilities and grid ranges at the top of
main() to match your own behavior or your own workday distribution.
"""

from __future__ import annotations

import argparse
import itertools
import time

import numpy as np

LUNCH_START = 12 * 60
LUNCH_END = 13 * 60

# Per-minute press probabilities per LED phase.
P_PRESS_YELLOW = 0.015
P_PRESS_RED = 0.040
P_PRESS_FAST_RED = 0.060
# One-shot bumps when the on-screen spiral animations fire (10 / 5 min before
# death). Independent of LED-phase boundaries — the firmware fires those at
# fixed offsets from death.
P_WARN10_BUMP = 0.04
P_WARN5_BUMP = 0.03


def sample_days(n: int, rng: np.random.Generator) -> tuple[np.ndarray, np.ndarray]:
    s = (rng.triangular(8.0, 8.5, 9.5, n) * 60).astype(np.int32)
    e = (rng.triangular(16.0, 17.5, 19.0, n) * 60).astype(np.int32)
    return s, e


def simulate(
    rt: int,
    yellow_at: int,
    red_at: int,
    fast_red_at: int,
    s: np.ndarray,
    e: np.ndarray,
    rng: np.random.Generator,
) -> dict:
    n = len(s)
    expire = s + rt

    presses = np.zeros(n, dtype=np.int32)
    slept_work = np.zeros(n, dtype=np.int32)
    slept_lunch = np.zeros(n, dtype=np.int32)
    after_hours = np.zeros(n, dtype=np.int32)

    t_min = int(s.min())
    t_max = int(max(e.max(), expire.max())) + 1

    for t in range(t_min, t_max):
        # Free re-tap when the user re-logs in at 13:00.
        if t == LUNCH_END:
            in_workday = (t >= s) & (t < e)
            expire = np.where(in_workday, t + rt, expire)

        in_workday = (t >= s) & (t < e)
        at_lunch = LUNCH_START <= t < LUNCH_END
        device_running = t < expire
        device_dead = ~device_running

        if at_lunch:
            slept_lunch += (in_workday & device_dead).astype(np.int32)
        else:
            slept_work += (in_workday & device_dead).astype(np.int32)

        past_end = (t >= e) & device_running
        after_hours += past_end.astype(np.int32)

        if not at_lunch:
            eligible = in_workday & device_running
            if eligible.any():
                remaining = expire - t
                p = np.zeros(n, dtype=np.float32)
                yellow = (remaining > red_at) & (remaining <= yellow_at)
                red = (remaining > fast_red_at) & (remaining <= red_at)
                fast_red = (remaining > 0) & (remaining <= fast_red_at)
                p[yellow] = P_PRESS_YELLOW
                p[red] = P_PRESS_RED
                p[fast_red] = P_PRESS_FAST_RED
                p[remaining == 10] += P_WARN10_BUMP
                p[remaining == 5] += P_WARN5_BUMP

                roll = rng.random(n).astype(np.float32)
                press = eligible & (roll < p)
                np.putmask(expire, press, t + rt)
                presses += press.astype(np.int32)

    return {
        "rt": rt,
        "yellow_at": yellow_at,
        "red_at": red_at,
        "fast_red_at": fast_red_at,
        "presses": presses,
        "slept_work": slept_work,
        "slept_lunch": slept_lunch,
        "after_hours": after_hours,
    }


def summarize(r: dict) -> dict:
    sw = r["slept_work"]
    sl = r["slept_lunch"]
    ah = r["after_hours"]
    pr = r["presses"]
    sweet = (sl >= 15) & (sl <= 45)
    return {
        "rt": r["rt"],
        "yellow_at": r["yellow_at"],
        "red_at": r["red_at"],
        "fast_red_at": r["fast_red_at"],
        "p_sweet": sweet.mean(),
        "p_lunch_any": (sl > 0).mean(),
        "p_no_work_sleep": (sw == 0).mean(),
        "mean_lunch": sl.mean(),
        "mean_work_sleep": sw.mean(),
        "mean_presses": pr.mean(),
        "mean_after": ah.mean(),
    }


def score(r: dict) -> float:
    return (
        r["p_sweet"]
        + 0.5 * r["p_lunch_any"]
        - 1.5 * (1 - r["p_no_work_sleep"])
        - 0.05 * r["mean_after"] / 60
    )


def fmt_h(m: float) -> str:
    m = int(round(m))
    h, mm = divmod(m, 60)
    return f"{h}h{mm:02d}m" if h else f"{mm}m"


def fmt_rt(m: int) -> str:
    h, mm = divmod(int(m), 60)
    return f"{h}h{mm:02d}m"


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--n", type=int, default=50_000, help="days per combo")
    ap.add_argument("--seed", type=int, default=2026)
    args = ap.parse_args()

    # 4-D search grid. Wider/finer is more honest; narrower is faster.
    rt_range = list(range(230, 251, 5))         # 230..250 step 5  (5)
    yellow_at_range = list(range(20, 71, 5))    #  20..70 step 5   (11)
    red_at_range = list(range(10, 41, 5))       #  10..40 step 5   (7)
    fast_red_at_range = list(range(4, 21, 2))   #   4..20 step 2   (9)

    print(f"tune_runtime — N={args.n} days/combo")
    print(f"  RT          {rt_range[0]}..{rt_range[-1]} step 5  ({len(rt_range)})")
    print(f"  YELLOW_AT   {yellow_at_range[0]}..{yellow_at_range[-1]} step 5  ({len(yellow_at_range)})")
    print(f"  RED_AT      {red_at_range[0]}..{red_at_range[-1]} step 5  ({len(red_at_range)})")
    print(f"  FAST_RED_AT {fast_red_at_range[0]}..{fast_red_at_range[-1]} step 2  ({len(fast_red_at_range)})")
    print(f"  press: yellow {P_PRESS_YELLOW}/min, red {P_PRESS_RED}/min, "
          f"fast {P_PRESS_FAST_RED}/min")
    print()

    rng = np.random.default_rng(args.seed)
    s, e = sample_days(args.n, rng)

    combos = [
        (rt, ya, ra, fra)
        for rt, ya, ra, fra in itertools.product(
            rt_range, yellow_at_range, red_at_range, fast_red_at_range
        )
        if ya > ra > fra > 0
    ]
    print(f"  {len(combos)} valid combos to evaluate...")
    t0 = time.time()

    results = []
    for i, (rt, ya, ra, fra) in enumerate(combos):
        sim_rng = np.random.default_rng(args.seed + 1 + i)
        r = simulate(rt, ya, ra, fra, s, e, sim_rng)
        results.append(summarize(r))
        if (i + 1) % 200 == 0:
            elapsed = time.time() - t0
            rate = (i + 1) / elapsed
            eta = (len(combos) - i - 1) / rate
            print(f"  ... {i+1}/{len(combos)}  ({rate:.1f}/sec, ETA {eta:.0f}s)")

    print(f"  done in {time.time() - t0:.0f}s")
    print()

    by_score = sorted(results, key=lambda r: -score(r))[:25]
    print("=== top 25 by composite score ===")
    print(f"{'RT':>6} {'YEL':>4} {'RED':>4} {'FST':>4} | "
          f"{'p_sweet':>7} {'p_any':>6} {'p_no_fail':>9} | "
          f"{'lunch':>5} {'work':>4} {'press':>5} {'score':>6}")
    print("-" * 86)
    for r in by_score:
        print(f"{fmt_rt(r['rt']):>6} {r['yellow_at']:>4} {r['red_at']:>4} {r['fast_red_at']:>4} | "
              f"{r['p_sweet']*100:>6.1f}% {r['p_lunch_any']*100:>5.1f}% "
              f"{r['p_no_work_sleep']*100:>8.1f}% | "
              f"{fmt_h(r['mean_lunch']):>5} {fmt_h(r['mean_work_sleep']):>4} "
              f"{r['mean_presses']:>5.2f} {score(r):>6.3f}")

    # Where does the firmware's currently-shipping combo land?
    shipping = next(
        (r for r in results
         if r["rt"] == 240 and r["yellow_at"] == 30
         and r["red_at"] == 25 and r["fast_red_at"] == 20),
        None,
    )
    if shipping is not None:
        rank = 1 + sum(1 for r in results if score(r) > score(shipping))
        print()
        print("=== current firmware (RT=4h00 YEL=30 RED=25 FST=20) ===")
        print(f"   p_sweet={shipping['p_sweet']*100:.1f}%  "
              f"p_any={shipping['p_lunch_any']*100:.1f}%  "
              f"p_no_fail={shipping['p_no_work_sleep']*100:.1f}%  "
              f"score={score(shipping):.3f}")
        print(f"   rank = {rank} / {len(results)}")

    best = by_score[0]
    print()
    print(f"PICK: RT={fmt_rt(best['rt'])}  YELLOW_AT={best['yellow_at']}  "
          f"RED_AT={best['red_at']}  FAST_RED_AT={best['fast_red_at']}")
    print(f"   P(sweet 12:15-12:45) = {best['p_sweet']*100:.1f}%")
    print(f"   P(any lunch sleep)   = {best['p_lunch_any']*100:.1f}%")
    print(f"   P(no work fail)      = {best['p_no_work_sleep']*100:.1f}%")
    print(f"   mean lunch dead      = {fmt_h(best['mean_lunch'])}")
    print(f"   mean work sleep      = {fmt_h(best['mean_work_sleep'])}")
    print(f"   mean presses         = {best['mean_presses']:.2f}/day")
    print(f"   mean after-hrs       = {fmt_h(best['mean_after'])}")


if __name__ == "__main__":
    main()
