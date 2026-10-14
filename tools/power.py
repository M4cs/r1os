"""What the R1 used between two snapshots (tools/r1h power, T-032): battery, CPU per process, suspends and wakeups.

    python tools/power.py <before> <after> [--json]

A snapshot is the device's own counters, one per line (tools/r1h writes them):
  T <uptime s>                      B <charge µAh> <current µA> <voltage µV> <capacity %> <full µAh> <status>
  S <suspends>                      D mWakefulness=<Awake | Asleep | Dozing> mScreenState=<display 0: ON | OFF | DOZE>
  G <coulombs> <current>            MediaTek's fuel gauge: its coulomb counter (0.1 mAh, unverified), which keeps counting
                                    at 100% where the charge counter sits at full; its current (0.1 mA, unverified)
  C cpu <user> <nice> <system> <idle> <iowait> <irq> <softirq> …   (jiffies, all cores)
  W <wakeup_count> <event_count> <name>                            (/sys/class/wakeup)
  P <pid> (<comm>) <state> <ppid> … <utime> <stime> …              (/proc/<pid>/stat)
"""
import json
import sys

HZ = 100  # USER_HZ on the R1 (getconf CLK_TCK)


def read(path):
    s = {"procs": {}, "wake": {}, "cpu": None}
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            line = line.rstrip("\r\n")
            kind, _, rest = line.partition(" ")
            if kind == "T":
                s["t"] = float(rest.split()[0])
            elif kind == "B":
                p = rest.split()
                s["charge"], s["current"], s["voltage"], s["capacity"], s["full"], s["status"] = int(p[0]), int(p[1]), int(p[2]), int(p[3]), int(p[4]), " ".join(p[5:])
            elif kind == "S":
                s["suspends"] = int(rest or 0)
            elif kind == "G":
                p = rest.split()
                if len(p) >= 1 and p[0].lstrip("-").isdigit():
                    s["coulomb"] = int(p[0])
            elif kind == "D":
                # The R1's own screen (display 0's mScreenState), else Android's wakefulness: that stays Awake while
                # the app window's own power group is awake, with the R1's screen off (T-113).
                f = dict(x.split("=", 1) for x in rest.split() if "=" in x)
                s["screen"] = f.get("mScreenState") or f.get("mWakefulness")
            elif kind == "C":
                s["cpu"] = [int(x) for x in rest.split()[1:]]
            elif kind == "W":
                p = rest.split(None, 2)
                if len(p) == 3:
                    s["wake"][p[2]] = s["wake"].get(p[2], 0) + int(p[0])
            elif kind == "P":
                # comm can hold spaces and brackets: split on the last ")".
                head, _, tail = rest.rpartition(")")
                pid, _, comm = head.partition(" (")
                f = tail.split()
                if len(f) < 13:
                    continue
                ppid, utime, stime = int(f[1]), int(f[11]), int(f[12])
                name = "kernel threads" if ppid == 2 or pid == "2" else comm
                s["procs"][pid] = (name, utime + stime)
    return s


def main():
    # Windows' console isn't UTF-8 by default: "·" and "→" would fail.
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    as_json = "--json" in sys.argv
    a, b = read(args[0]), read(args[1])
    secs = max(b["t"] - a["t"], 0.001)
    used_mah = (a["charge"] - b["charge"]) / 1000.0
    avg_ma = used_mah / (secs / 3600.0)
    by = {}
    for pid, (name, t) in b["procs"].items():
        before = a["procs"].get(pid)
        d = t - before[1] if before and before[0] == name else t
        if d > 0:
            by[name] = by.get(name, 0) + d
    top = sorted(by.items(), key=lambda x: -x[1])[:12]
    # All cores busy: the processes' own CPU time over the window (/proc/stat's totals jump when cores go offline
    # in deep sleep, and came out negative over a 30-minute unplug).
    cores = 8
    cpu_pct = 100.0 * sum(by.values()) / HZ / (secs * cores)
    gauge_mah = (a["coulomb"] - b["coulomb"]) / 10.0 if "coulomb" in a and "coulomb" in b else None
    wake = {n: c - a["wake"].get(n, 0) for n, c in b["wake"].items() if c - a["wake"].get(n, 0) > 0}
    wtop = sorted(wake.items(), key=lambda x: -x[1])[:8]
    out = {
        "seconds": round(secs, 1),
        "screen": [a.get("screen"), b.get("screen")],
        "status": [a.get("status"), b.get("status")],
        "capacity": [a.get("capacity"), b.get("capacity")],
        "used_mah": round(used_mah, 2),
        "gauge_mah": None if gauge_mah is None else round(gauge_mah, 1),
        "gauge_ma": None if gauge_mah is None else round(gauge_mah / (secs / 3600.0), 1),
        "avg_ma": round(avg_ma, 1),
        "per_hour_pct": round(100.0 * avg_ma / max(b["full"] / 1000.0, 1), 2),
        "full_mah": round(b["full"] / 1000.0),
        "current_ma": [round(a["current"] / 1000.0, 1), round(b["current"] / 1000.0, 1)],
        "voltage": round(b["voltage"] / 1e6, 3),
        "cpu_pct": round(cpu_pct, 1),
        "cpu_s": {n: round(t / HZ, 2) for n, t in top},
        "suspends": b.get("suspends", 0) - a.get("suspends", 0),
        "wakeups": sum(wake.values()),
        "wakeups_by": dict(wtop),
    }
    if as_json:
        print(json.dumps(out, indent=2))
        return
    print(f"r1h power · {out['seconds']:.0f} s · screen {' → '.join(dict.fromkeys(x for x in out['screen'] if x))} · battery {b.get('status')} {b.get('capacity')}%")
    charging = "Charging" in (a.get("status"), b.get("status")) or "Full" in (a.get("status"), b.get("status"))
    print(f"  battery: {out['used_mah']:.2f} mAh used, {out['avg_ma']:.1f} mA average (~{out['per_hour_pct']:.1f}% an hour)"
          f"{' · on the charger: not a drain figure' if charging else ''}")
    if gauge_mah is not None:
        print(f"           the fuel gauge's coulombs: {out['gauge_mah']:.1f} mAh used, {out['gauge_ma']:.1f} mA average (its units unverified)")
    print(f"           now {out['current_ma'][1]:+.1f} mA (+ into the battery), {out['voltage']:.2f} V")
    print(f"  cpu: {out['cpu_pct']:.1f}% of all cores busy; by process (seconds of one core, % of the window):")
    for n, t in out["cpu_s"].items():
        print(f"    {n:<22} {t:7.2f} s  {100.0 * t / secs:5.1f}%")
    print(f"  suspends: {out['suspends']} · wakeups: {out['wakeups']}" + (f" ({', '.join(f'{n} {c}' for n, c in wtop)})" if wtop else ""))


if __name__ == "__main__":
    main()
