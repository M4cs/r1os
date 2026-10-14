"""The R1's app-build eval suite: builds each brief on the R1 and reports how it went.

Usage: py -3 tools/eval-apps.py [--keep] [--model haiku|sonnet|opus|codex] [--brain claude|codex] [--only 1,3] [--edits] [briefs.json]
--brain codex is --model codex: the builds go through Codex (T-045). --edits runs the edit briefs
(fixtures/evals/edits.json) instead: each installs a fixture app under its own id, puts some data in it through
its actions, asks for the change (`POST /api/apps/{id}/edit`), and checks the change built and the data survived.

For each brief (fixtures/evals/briefs.json by default) it starts a build through r1d (`POST /api/create`, as a
chat build would), follows its stages, then checks the app it made: that it installed, which permissions it
asks for against what the brief needs, and what the build had to repair on the way. Then it removes the app
(to the trash, for 7 days) unless --keep.

Every build runs in the agent box on the R1 and costs a little on the user's subscription, so it asks first
(YES=1 skips the question). Needs r1d forwarded on 127.0.0.1:7172: any tools/r1h call sets that up.
Writes artifacts/evals/<time>.md and .json (git-ignored).
"""
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request

B = "http://127.0.0.1:7172"
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BUILD_TIMEOUT_S = 300


def token():
    """r1d's per-boot token (ADR-0026), read as root over adb."""
    env = dict(os.environ, MSYS_NO_PATHCONV="1")
    for cmd in (["adb", "shell", "cat", "/data/r1os/run/token"], ["adb", "shell", "su", "0", "cat", "/data/r1os/run/token"]):
        out = subprocess.run(cmd, capture_output=True, text=True, env=env).stdout.strip()
        if out and "No such file" not in out and "denied" not in out:
            return out
    return ""


T = token()


def call(method, path, body=None, timeout=30):
    headers = {"Content-Type": "application/json"}
    if T:
        headers["Authorization"] = "Bearer " + T
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(B + path, method=method, data=data, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, json.loads(r.read() or b"{}")
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b"{}")


def last_seq():
    _, ev = call("GET", "/harness/events?after=0")
    return ev[-1]["seq"] if ev else 0


def start(path, body):
    """Starts a build (waiting while another runs). The build's id, or None and the error."""
    while True:
        status, r = call("POST", path, body)
        if status != 409:
            break
        time.sleep(5)  # another build is running
    return r.get("build"), r.get("error", f"HTTP {status}")


def build(brief, model, big=False):
    """Starts one build and follows it. Returns what happened."""
    seq = last_seq()
    t0 = time.time()
    body = {"brief": brief}
    if model:
        body["model"] = model
    if big:
        body["big"] = True
    bid, err = start("/api/create", body)
    if bid is None:
        return {"ok": False, "error": err, "seconds": 0, "stages": [], "problems": []}
    return follow(bid, seq, t0)


def follow(bid, seq, t0):
    """Follows build `bid`'s events to its end."""
    out = {"ok": False, "build": bid, "stages": [], "problems": [], "seconds": None, "cost_usd": None}
    while time.time() - t0 < BUILD_TIMEOUT_S:
        _, events = call("GET", f"/harness/events?after={seq}")
        for e in events:
            seq = e["seq"]
            if e.get("type") != "build" or e.get("id") != bid:
                continue
            stage = e.get("stage")
            if not out["stages"] or out["stages"][-1] != stage:
                out["stages"].append(stage)
            out["problems"] += e.get("problems") or []
            for k in ("app", "seconds", "cost_usd", "error", "name"):
                if e.get(k) is not None:
                    out[k] = e[k]
            if stage in ("done", "failed", "stopped"):
                out["ok"] = stage == "done"
                out.setdefault("seconds", round(time.time() - t0, 1))
                return out
        time.sleep(1)
    out["error"] = f"no result in {BUILD_TIMEOUT_S} s"
    return out


def rows(app, query):
    """How many rows one of an app's named queries returns."""
    status, r = call("POST", f"/api/apps/{app}/query", {"name": query})
    if status != 200:
        return None
    if isinstance(r, list):
        return len(r)
    return len(r.get("rows", r.get("data", [])))


def edit(case, model):
    """One edit brief: the fixture installed as its own app, data put in, the change asked for. What happened."""
    app = case.get("as", "eval-app")
    src = open(os.path.join(ROOT, case["app"]), encoding="utf-8").read()
    src = re.sub(r"(app:\s*\{\s*id:\s*)[\w-]+", lambda m: m.group(1) + app, src, count=1)
    status, r = call("POST", "/api/apps", {"source": src})
    if status != 200:
        return {"ok": False, "app": None, "error": f"install: {r.get('error') or r.get('diagnostics') or status}", "stages": [], "problems": [], "notes": ["didn't install"]}
    for s in case.get("seed", []):
        call("POST", f"/api/apps/{app}/run", {"action": s["action"], "params": s.get("params", {})})
    q = case["rows"]["query"]
    before = rows(app, q)
    seq = last_seq()
    t0 = time.time()
    body = {"request": case["request"]}
    if model:
        body["model"] = model
    bid, err = start(f"/api/apps/{app}/edit", body)
    if bid is None:
        return {"ok": False, "app": app, "error": err, "stages": [], "problems": [], "notes": [f"didn't start: {err}"]}
    out = follow(bid, seq, t0)
    out["app"] = app
    out["rows_before"] = before
    out["rows_after"] = rows(app, q) if out["ok"] else None
    _, h = call("GET", f"/api/apps/{app}/history")
    out["versions"] = len(h.get("versions", [])) if isinstance(h, dict) else None
    notes = []
    if not out["ok"]:
        notes.append("didn't build: " + str(out.get("error", "failed")))
    else:
        if before != case["rows"].get("count", before):
            notes.append(f"the seed made {before} rows, not {case['rows']['count']}")
        if out["rows_after"] != before:
            notes.append(f"data: {before} rows before, {out['rows_after']} after")
        if (out["versions"] or 0) < 2:
            notes.append("no new version")
    out["notes"] = notes
    return out


def run_edits(cases, model, keep):
    """The edit briefs, one after another. Their results."""
    results = []
    for n, c in enumerate(cases, 1):
        print(f"[{n}/{len(cases)}] {c['name']}: {c['request']}", flush=True)
        r = edit(c, model)
        r["brief"] = f"{c['name']}: {c['request']}"
        print(f"    {'ok' if r['ok'] else 'FAILED'} in {r.get('seconds')} s; rows {r.get('rows_before')} -> {r.get('rows_after')}; "
              f"{', '.join(r['notes']) or 'as expected'}", flush=True)
        if r.get("app") and not keep:
            call("DELETE", f"/api/apps/{r['app']}")
        results.append(r)
    return results


def check(result, expect):
    """What's wrong with the app it made, against what the brief needs."""
    notes = []
    app = result.get("app")
    if not result["ok"] or not app:
        return ["didn't build: " + str(result.get("error", "failed"))]
    status, r = call("GET", f"/api/apps/{app}")
    if status != 200:
        return [f"not installed (HTTP {status})"]
    perms = sorted({p.get("id") for p in r["app"].get("permissions", [])})
    result["permissions"] = perms
    for p in expect.get("perms", []):
        if p not in perms:
            notes.append(f"doesn't ask for {p}")
    for p in expect.get("no_perms", []):
        if p in perms:
            notes.append(f"asks for {p} it doesn't need")
    source = r.get("source", "")
    result["spec_lines"] = source.count("\n")
    # The R1's apps take words from the agent, not typing (prompts/design.md): no `input` or `form` blocks.
    if re.search(r"^\s*(-\s*)?(input|form)\s*:", source, re.M):
        notes.append("has a text field")
    # Blocks the brief calls for (a `wheel` for values it sets, a `game`; ADR-0038).
    for b in expect.get("blocks", []):
        if not re.search(rf"^\s*(-\s*)?{b}\s*:", source, re.M):
            notes.append(f"has no {b}")
    # Steps it needs (`play` for audio, ADR-0040), and the logic lane (ADR-0041): needed, or just used.
    for s in expect.get("steps", []):
        if not re.search(rf"(^|[\s{{\[,-])\s*{s}\s*:", source, re.M):
            notes.append(f"has no {s} step")
    result["logic"] = bool(r["app"].get("logic"))
    if expect.get("logic") and not result["logic"]:
        notes.append("has no logic")
    # A widget (ADR-0042): a glance card, on the glance screen and drawn (a web one waits for its permission there).
    result["glance"] = bool(r["app"].get("glance"))
    if expect.get("glance"):
        if not result["glance"]:
            notes.append("has no glance card")
        else:
            _, g = call("GET", "/api/glance")
            w = next((w for w in (g or {}).get("widgets", []) if w.get("app") == app), None)
            if not w:
                notes.append("isn't on the glance screen")
            elif not w.get("card") and "net.http" not in perms:
                notes.append(f"its card doesn't draw ({w.get('note')})")
    return notes


def main():
    args = sys.argv[1:]
    keep = "--keep" in args
    edits = "--edits" in args
    model = None
    only = None
    path = os.path.join(ROOT, "fixtures", "evals", "edits.json" if edits else "briefs.json")
    i = 0
    while i < len(args):
        a = args[i]
        if a == "--model":
            model, i = args[i + 1], i + 1
        elif a == "--brain":
            model = None if args[i + 1] == "claude" else args[i + 1]
            i += 1
        elif a == "--only":
            only = {int(n) for n in args[i + 1].split(",")}
            i += 1
        elif a not in ("--keep", "--edits"):
            path = a
        i += 1
    briefs = json.load(open(path, encoding="utf-8"))
    if only:
        briefs = [b for n, b in enumerate(briefs, 1) if n in only]
    if not T:
        print("No r1d token: is the R1 on adb, with r1d running?", file=sys.stderr)
    if os.environ.get("YES") != "1":
        answer = input(f"{'Edit' if edits else 'Build'} {len(briefs)} apps on the R1 ({model or 'its build model'}); each costs a little. Go? [y/N] ")
        if answer.strip().lower() not in ("y", "yes"):
            return

    results = run_edits(briefs, model, keep) if edits else []
    for n, b in enumerate([] if edits else briefs, 1):
        print(f"[{n}/{len(briefs)}] {b['brief']}", flush=True)
        r = build(b["brief"], model, b.get("big", False))
        r["brief"] = b["brief"]
        r["notes"] = check(r, b.get("expect", {}))
        uses = "".join(f" (with {u})" for u, on in (("logic", r.get("logic")), ("a glance card", r.get("glance"))) if on)
        print(f"    {'ok' if r['ok'] else 'FAILED'} in {r.get('seconds')} s{uses}; {', '.join(r['notes']) or 'as expected'}", flush=True)
        if r.get("app") and not keep:
            call("DELETE", f"/api/apps/{r['app']}")
        results.append(r)

    built = sum(r["ok"] for r in results)
    clean = sum(r["ok"] and not r["notes"] for r in results)
    secs = [r["seconds"] for r in results if r["ok"] and r.get("seconds")]
    cost = sum(r.get("cost_usd") or 0 for r in results)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    out_dir = os.path.join(ROOT, "artifacts", "evals")
    os.makedirs(out_dir, exist_ok=True)
    lines = [
        f"# App {'edit' if edits else 'build'} evals, {time.strftime('%Y-%m-%d %H:%M')}",
        "",
        f"Model: {model or 'the R1 setting'}. Built {built}/{len(results)}, as expected {clean}/{len(results)}, "
        f"median {sorted(secs)[len(secs) // 2] if secs else '-'} s, ${cost:.2f} in all.",
        "",
        "| # | brief | result | s | repairs | permissions | logic | glance | notes |",
        "|---|---|---|---|---|---|---|---|---|",
    ]
    if edits:
        lines[4:6] = ["| # | edit | result | s | repairs | rows before → after | versions | notes |", "|---|---|---|---|---|---|---|---|"]
    for n, r in enumerate(results, 1):
        if edits:
            lines.append(f"| {n} | {r['brief']} | {'ok' if r['ok'] else 'failed'} | {r.get('seconds') or '-'} | {len(r['problems'])} | "
                         f"{r.get('rows_before')} → {r.get('rows_after')} | {r.get('versions')} | {'; '.join(r['notes']) or '-'} |")
            continue
        lines.append(
            f"| {n} | {r['brief']} | {'ok' if r['ok'] else 'failed'} | {r.get('seconds') or '-'} | {len(r['problems'])} | "
            f"{', '.join(r.get('permissions', [])) or '-'} | {'yes' if r.get('logic') else '-'} | {'yes' if r.get('glance') else '-'} | "
            f"{'; '.join(r['notes']) or '-'} |"
        )
    md = os.path.join(out_dir, f"{stamp}.md")
    open(md, "w", encoding="utf-8").write("\n".join(lines) + "\n")
    json.dump(results, open(os.path.join(out_dir, f"{stamp}.json"), "w", encoding="utf-8"), indent=2)
    print(f"\nBuilt {built}/{len(results)}, as expected {clean}/{len(results)}. Report: {md}")


if __name__ == "__main__":
    main()
