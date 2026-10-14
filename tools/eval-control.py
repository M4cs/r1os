"""App and browser control evals (T-144): real agent turns on the R1 doing the scenarios in fixtures/evals/control.json
in the R1's browser and the app window, scored for success, time and steps.

    py -3 tools/eval-control.py [--only b1,u2] [--agent archer] [--keep] [--list]

Each scenario runs in a test chat of its own, marked a dry run (POST /harness/dry_run): nothing that adds to a cart,
orders, pays or sends is pressed in it. The R1's cards for those chats are answered here: a first tap or typing is
allowed, the first use of an app too, and every final step is declined. The app window and the browser are closed
before each scenario. Writes artifacts/evals/control-<time>.json and .md, and deletes the chats unless --keep.

Each scenario costs a little on the user's subscription: it asks first (YES=1 skips). Needs r1d forwarded on 127.0.0.1:7172
(any tools/r1h call sets that up) and adb as root.
"""
import json
import os
import re
import subprocess
import sys
import threading
import time
import urllib.request

B = "http://127.0.0.1:7172"
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ENV = dict(os.environ, MSYS_NO_PATHCONV="1")


def adb(*args, binary=False):
    out = subprocess.run(["adb", *args], capture_output=True, env=ENV)
    return out.stdout if binary else out.stdout.decode("utf-8", "replace")


def token():
    for cmd in (["shell", "cat", "/data/r1os/run/token"], ["shell", "su", "0", "cat", "/data/r1os/run/token"]):
        t = adb(*cmd).strip()
        if t and "No such file" not in t and "denied" not in t:
            return t
    return ""


T = token()


def call(method, path, body=None, timeout=60):
    headers = {"Content-Type": "application/json", "Authorization": "Bearer " + T}
    req = urllib.request.Request(B + path, method=method, data=None if body is None else json.dumps(body).encode(), headers=headers)
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read() or b"{}")


def last_seq():
    ev = call("GET", "/harness/events?after=0")
    return ev[-1]["seq"] if ev else 0


class Awake(threading.Thread):
    """The R1's screen kept on while a scenario runs: the app window's display only stays awake while it is."""

    def __init__(self):
        super().__init__(daemon=True)
        self.stop = threading.Event()
        self.paused = threading.Event()
        # The user's hands on the R1 (Hands): its keys would mix with theirs.
        self.hands = threading.Event()

    def run(self):
        while not self.stop.is_set():
            if not self.paused.is_set() and not self.hands.is_set():
                adb("shell", "input", "keyevent", "224")
                adb("shell", "input", "keyevent", "59")
            self.stop.wait(8)


class Hands:
    """Whether the user is using the R1: its wheel or button (r1d's hardware events) within the last QUIET seconds. A test's
    taps and keys mix with theirs, and its load made the R1 lag under their hands (2026-10-13)."""

    QUIET = 120
    INPUT = ("wheel", "button", "gesture")

    def __init__(self):
        self.last = 0.0  # when they last touched it, on this PC's clock

    def saw(self, events):
        """Notes the user's input among r1d's events (their t_ms is r1d's clock since it started: aged against its uptime)."""
        if not events:
            return
        try:
            newest = call("GET", "/api/state")["uptime_s"] * 1000
        except Exception:
            newest = max(e.get("t_ms", 0) for e in events)
        for e in events:
            if e.get("source") == "hw" and e.get("type") in self.INPUT:
                self.last = max(self.last, time.time() - (newest - e.get("t_ms", newest)) / 1000)

    def busy(self):
        return time.time() - self.last < self.QUIET

    def wait(self, awake):
        """Before a scenario: while they're using it, the screen's theirs (no keys from the runner) and the next test waits."""
        seq = last_seq()
        told = False
        while True:
            events = call("GET", f"/harness/events?after={seq}")
            if events:
                seq = events[-1]["seq"]
            self.saw(events)
            if not self.busy():
                break
            if not told:
                print(f"   you're using the R1: waiting until it's been {self.QUIET} s since your last input", flush=True)
                told = True
            awake.hands.set()
            time.sleep(5)
        awake.hands.clear()


def battery():
    """The R1's battery level (%), or None if it can't tell."""
    m = re.search(r"^\s*level: (\d+)", adb("shell", "dumpsys", "battery"), re.M)
    return int(m.group(1)) if m else None


# Below LOW before a scenario, the runner lets the screen sleep and waits for RESUME. On a PC's USB port (500 mA) the
# R1 loses about 1% every 2.5 minutes while agents work: the night of 2026-10-13 it went from 100% to empty in five and
# a half hours and shut itself down mid-run.
LOW, RESUME = 20, 50


def recharge(awake):
    level = battery()
    if level is None or level >= LOW:
        return
    print(f"   battery {level}%: screen off until {RESUME}%", flush=True)
    awake.paused.set()
    adb("shell", "input", "keyevent", "223")
    while (level := battery()) is not None and level < RESUME:
        time.sleep(60)
    print(f"   battery {level}%: going on", flush=True)
    awake.paused.clear()


def answer_cards(chat, log):
    """Answers the cards of this test chat: acting and an app's first use allowed, every final step declined."""
    for a in call("GET", "/api/approvals").get("approvals", []):
        if a.get("chat") != chat:
            continue
        title = a.get("title", "")
        first_use = title.startswith("use ") and title.endswith(" for you")
        allow = a.get("area") in ("android", "browser") and (a.get("always") or first_use)
        call("POST", f"/api/approvals/{a['id']}", {"allow": bool(allow), "always": False})
        log.append({"card": title, "area": a.get("area"), "allowed": bool(allow)})


def transcript(agent, session):
    """The CLI's own record of the turn: each tool call and whether it failed. Without a session (a turn that failed
    before r1d kept it), the agent's newest."""
    if session:
        path = adb("shell", f"find /data/r1os/agent/agents/{agent} -name '{session}.jsonl' 2>/dev/null").strip().splitlines()
    else:
        path = adb("shell", f"ls -t $(find /data/r1os/agent/agents/{agent} -name '*.jsonl' 2>/dev/null) 2>/dev/null").strip().splitlines()[:1]
    if not path:
        return []
    raw = adb("exec-out", f"cat {path[0]}", binary=True).decode("utf-8", "replace")
    steps = []
    for line in raw.splitlines():
        try:
            e = json.loads(line)
        except ValueError:
            continue
        for x in (e.get("message") or {}).get("content") or []:
            if not isinstance(x, dict):
                continue
            if x.get("type") == "tool_use":
                steps.append({"tool": x["name"].replace("mcp__r1__", ""), "input": json.dumps(x.get("input"))[:200], "at": e.get("timestamp", "")})
            elif x.get("type") == "tool_result":
                c = x.get("content")
                said = " ".join(y.get("text", "") for y in c if isinstance(y, dict) and y.get("type") == "text") if isinstance(c, list) else str(c)
                for s in reversed(steps):
                    if "result" not in s:
                        s["result"] = said[:300]
                        s["error"] = bool(x.get("is_error"))
                        break
    return steps


def cart_count():
    """How many items Uber Eats' cart holds, from its tab bar ("Carts tab, 4 of 5 · 2"); None if it can't tell."""
    try:
        call("POST", "/harness/app_window", {"op": "close", "args": {}})
        for _ in range(3):
            r = call("POST", "/harness/app_window", {"op": "open", "args": {"package": "com.ubercab.eats", "name": "Uber Eats"}}, timeout=90)
            text = (r.get("result") or {}).get("text", "") if isinstance(r.get("result"), dict) else ""
            if "Carts tab" not in text:
                time.sleep(3)
                r = call("POST", "/harness/app_window", {"op": "read", "args": {}}, timeout=60)
                text = (r.get("result") or {}).get("text", "") if isinstance(r.get("result"), dict) else ""
            m = re.search(r"Carts tab, \d of \d(?: · (\d+))?\"", text)
            if m:
                return int(m.group(1) or 0)
        return None
    finally:
        call("POST", "/harness/app_window", {"op": "close", "args": {}})


def run(sc, agent_id, agent_name, suffixes, rejects, keep, hands=None, awake=None):
    for surface in ("app_window", "browser"):
        try:
            call("POST", f"/harness/{surface}", {"op": "close", "args": {}})
        except Exception:
            pass
    # A food order in a test must never reach the cart: counted before and after (2026-10-13: one did).
    watch_cart = sc.get("suffix") == "dry"
    cart_before = cart_count() if watch_cart else None
    chat = call("POST", "/api/chats", {"agent": agent_id})["chat"]["id"]
    call("POST", "/harness/dry_run", {"chat": chat, "on": True})
    text = sc["task"] + suffixes.get(sc.get("suffix", ""), "")
    seq = last_seq()
    t0 = time.time()
    call("POST", f"/api/chats/{chat}/messages", {"text": text})
    cards, tools, first, reply = [], [], None, None
    deadline = t0 + sc.get("timeout", 900)
    while time.time() < deadline:
        answer_cards(chat, cards)
        events = call("GET", f"/harness/events?after={seq}")
        if hands is not None:
            # The user picked it up mid-test: the runner's keys stop until they're done (the agent's turn goes on).
            hands.saw(events)
            (awake.hands.set if hands.busy() else awake.hands.clear)()
        for e in events:
            seq = e["seq"]
            if e["type"] == "chat" and e.get("chat") == chat:
                if e["kind"] == "delta" and first is None:
                    first = time.time() - t0
                elif e["kind"] == "tool":
                    tools.append(e.get("text"))
                elif e["kind"] in ("done", "error"):
                    reply = (e["kind"], e.get("text", ""), time.time() - t0)
        if reply:
            break
        time.sleep(0.5)
    if reply is None:
        call("POST", f"/api/chats/{chat}/stop")
    kind, said, took = reply or ("timeout", "", time.time() - t0)
    verified = None
    if sc.get("verify_browser"):
        try:
            page = call("POST", "/harness/browser", {"op": "read", "args": {"filter": "all"}})
            lines = json.dumps(page)
            r = page.get("result")
            lines = (r.get("text", "") if isinstance(r, dict) else str(r)) or lines
            verified = {p: bool(re.search(p, lines, re.I)) for p in sc["verify_browser"]}
        except Exception as e:  # the browser's closed
            verified = {"(browser open)": False, "error": str(e)[:100]}
    session = call("GET", f"/api/chats/{chat}")["chat"].get("session")
    steps = transcript(agent_id, session)
    checks = {p: bool(re.search(p, said, re.I)) for p in sc.get("expect", [])}
    # Words that say it didn't get there ("I got partway"): run 4's sandwich passed on its words alone.
    rejected = [p for p in rejects if re.search(p, said, re.I)]
    ok = kind == "done" and all(checks.values()) and not rejected and (verified is None or all(v is True for v in verified.values()))
    if not keep:
        call("DELETE", f"/api/chats/{chat}")
    call("POST", "/harness/dry_run", {"chat": chat, "on": False})
    cart_after = cart_count() if watch_cart else None
    cart_grew = cart_before is not None and cart_after is not None and cart_after > cart_before
    if cart_grew:
        ok = False
        print(f"   !!! THE CART GREW: {cart_before} -> {cart_after}", flush=True)
    errors = [s for s in steps if s.get("error")]
    return {
        "id": sc["id"], "name": sc["name"], "surface": sc["surface"], "ok": ok, "kind": kind,
        "seconds": round(took, 1), "first_word": first and round(first, 1),
        "tool_calls": len(steps), "tool_errors": len(errors),
        "dry_refusals": sum(1 for s in steps if "test run" in (s.get("result") or "")),
        "checks": checks, "rejected": rejected, "verified": verified, "reply": said, "cards": cards, "cart": [cart_before, cart_after],
        "steps": steps, "chat": chat if keep else None, "agent": agent_name,
    }


def main():
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    args = sys.argv[1:]
    spec = json.load(open(os.path.join(ROOT, "fixtures", "evals", "control.json"), encoding="utf-8"))
    scenarios = spec["scenarios"]
    if "--list" in args:
        for s in scenarios:
            print(f"{s['id']:4} {s['surface']:8} {s['name']}")
        return
    if "--only" in args:
        want = set(args[args.index("--only") + 1].split(","))
        scenarios = [s for s in scenarios if s["id"] in want]
    agent_name = args[args.index("--agent") + 1] if "--agent" in args else "archer"
    agents = call("GET", "/api/agents")["agents"]
    agent = next((a for a in agents if a["id"] == agent_name or a["name"].lower() == agent_name.lower()), None)
    if agent is None:
        sys.exit(f"no agent {agent_name!r}")
    if os.environ.get("YES") != "1":
        if input(f"{len(scenarios)} scenarios as {agent['name']}, each a little on the subscription. Go? [y/N] ").strip().lower() != "y":
            return
    awake = Awake()
    awake.start()
    hands = Hands()
    hands.saw(call("GET", "/harness/events?after=0"))
    results = []
    try:
        for sc in scenarios:
            hands.wait(awake)
            recharge(awake)
            print(f"\n== {sc['id']} {sc['name']} (battery {battery()}%)", flush=True)
            rejects = spec.get("reject", {}).get(sc.get("suffix", ""), []) + sc.get("reject", [])
            r = run(sc, agent["id"], agent["name"], spec["suffix"], rejects, "--keep" in args, hands, awake)
            results.append(r)
            why = [f"missing {p}" for p, hit in r["checks"].items() if not hit] + [f"says {p}" for p in r["rejected"]]
            print(f"   {'PASS' if r['ok'] else 'FAIL'} {r['kind']} {r['seconds']} s, {r['tool_calls']} tools, {r['tool_errors']} errors, "
                  f"{r['dry_refusals']} dry refusals{'' if r['ok'] or not why else ' · ' + '; '.join(why)}\n   {r['reply'][:300]}", flush=True)
    finally:
        awake.stop.set()
        for surface in ("app_window", "browser"):
            try:
                call("POST", f"/harness/{surface}", {"op": "close", "args": {}})
            except Exception:
                pass
    stamp = time.strftime("%Y%m%d-%H%M%S")
    out = os.path.join(ROOT, "artifacts", "evals")
    os.makedirs(out, exist_ok=True)
    with open(os.path.join(out, f"control-{stamp}.json"), "w", encoding="utf-8") as f:
        json.dump(results, f, indent=2, ensure_ascii=False)
    passed = sum(r["ok"] for r in results)
    lines = [f"# Control evals {stamp}: {passed}/{len(results)} passed, as {agent['name']}", "",
             "| id | scenario | result | seconds | tools | errors | reply |", "|---|---|---|---|---|---|---|"]
    for r in results:
        lines.append(f"| {r['id']} | {r['name']} | {'pass' if r['ok'] else 'FAIL ' + r['kind']} | {r['seconds']} | {r['tool_calls']} | {r['tool_errors']} | {r['reply'][:120].replace('|', '/').replace(chr(10), ' ')} |")
    with open(os.path.join(out, f"control-{stamp}.md"), "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")
    print(f"\n{passed}/{len(results)} passed · artifacts/evals/control-{stamp}.md")


if __name__ == "__main__":
    main()
