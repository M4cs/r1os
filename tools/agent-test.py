"""Drives the R1's agent through r1d's API and prints what it did (tools, timings, replies).
Usage: py -3 tools/agent-test.py [codex] "message" ["message" ...]   (r1d forwarded on 127.0.0.1:7172)
       AGENT=<id or name> picks the agent (default: the one in focus).
"""
import json
import os
import sys
import time
import urllib.request

B = "http://127.0.0.1:7172"


def token():
    """r1d's per-boot token (ADR-0026), read as root over adb."""
    import os
    import subprocess
    env = dict(os.environ, MSYS_NO_PATHCONV="1")
    for cmd in (["adb", "shell", "cat", "/data/r1os/run/token"], ["adb", "shell", "su", "0", "cat", "/data/r1os/run/token"]):
        out = subprocess.run(cmd, capture_output=True, text=True, env=env).stdout.strip()
        if out and "No such file" not in out and "denied" not in out:
            return out
    return ""


T = token()


def call(method, path, body=None):
    headers = {"Content-Type": "application/json"}
    if T:
        headers["Authorization"] = "Bearer " + T
    req = urllib.request.Request(B + path, method=method, data=None if body is None else json.dumps(body).encode(),
                                 headers=headers)
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.loads(r.read() or b"{}")


def last_seq():
    ev = call("GET", "/harness/events?after=0")
    return ev[-1]["seq"] if ev else 0


def ask(chat, text, wait_build=True):
    seq = last_seq()
    t0 = time.time()
    call("POST", f"/api/chats/{chat}/messages", {"text": text})
    first = None
    tools, builds, reply = [], {}, None
    deadline = t0 + 240
    while time.time() < deadline:
        for e in call("GET", f"/harness/events?after={seq}"):
            seq = e["seq"]
            if e["type"] == "chat" and e.get("chat") == chat:
                if e["kind"] == "delta" and first is None:
                    first = time.time() - t0
                elif e["kind"] == "tool":
                    tools.append(e.get("text"))
                elif e["kind"] == "build":
                    builds[e.get("build")] = "started"
                elif e["kind"] in ("done", "error"):
                    reply = (e["kind"], e.get("text", ""), time.time() - t0)
            elif e["type"] == "build" and e.get("id") in builds:
                if e["stage"] in ("done", "failed"):
                    builds[e["id"]] = f"{e['stage']} {e.get('app', '')} said={e.get('said', '')!r} deleted={e.get('deleted')} {e.get('error', '')}"
        pending_build = wait_build and any(v == "started" for v in builds.values())
        if reply and not pending_build:
            break
        time.sleep(0.5)
    kind, text_, total = reply or ("timeout", "", time.time() - t0)
    print(f"\n> {text}\n  [{kind}] first word {first and round(first, 1)} s, done {round(total, 1)} s; tools: {tools}")
    for b, v in builds.items():
        print(f"  build {b}: {v}")
    print(f"  {text_[:400]}")


def main():
    # Replies can hold any language (a song called ラビットホール, T-113): Windows' console code page can't print them.
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    args = sys.argv[1:]
    brain = None
    if args and args[0] == "codex":
        brain, args = "codex", args[1:]
    if brain is None:
        call("POST", "/api/chats/warm", {})
        time.sleep(6)  # let the spare start, as it would while the user talks
    body = {"brain": brain} if brain else {}
    if os.environ.get("AGENT"):
        # r1d takes an id; a name is looked up (r1d falls back to the focused agent for one it doesn't know).
        want = os.environ["AGENT"]
        agents = call("GET", "/api/agents")["agents"]
        match = next((a for a in agents if a["id"] == want or a["name"].lower() == want.lower()), None)
        if match is None:
            sys.exit(f"no agent {want!r}: {', '.join(a['name'] for a in agents)}")
        body["agent"] = match["id"]
    chat = call("POST", "/api/chats", body)["chat"]["id"]
    print("chat", chat, brain or "claude")
    for text in args:
        ask(chat, text)


if __name__ == "__main__":
    main()
