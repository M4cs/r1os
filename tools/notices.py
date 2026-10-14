"""Third-party notices for what r1os ships (T-178, for releases: T-166): every package each binary or bundle holds, its
licence, and the licence texts the packages carry, one file each in out/notices/.

    py -3 tools/notices.py            (python3 elsewhere)

- **r1d** and **r1-bootlog** (the R1's, aarch64-linux-android) and **the companion** (Windows, its Rust and its window's
  npm packages): from `cargo tree`'s normal dependencies for that target, so build tools and proc-macros, which aren't
  linked in, are left out.
- **The shell** (served by r1d to the host's WebViews) and **the website**: their npm production dependencies, with
  their workspace packages' (`pnpm --filter <name>... licenses list --prod`).

Each text is written once, with every package that carries it. Read only: Cargo.lock, the cargo registry's sources and
node_modules. It prints what to look at before a release: copyleft licences, and packages with no licence text in them.
NOTICE.md covers what isn't a package: the fonts, the models, AOSP and the rest.
"""
import datetime
import json
import os
import re
import shutil
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "out", "notices")

# (output name, what it is, Rust package and target or None, npm package folders)
TARGETS = [
    ("r1d", "r1d, r1os's core daemon on the R1", ("r1d", "aarch64-linux-android"), []),
    ("r1-bootlog", "verbose boot's program on the R1", ("r1-bootlog", "aarch64-linux-android"), []),
    # The companion's window and the website take the shell's (and the site the companion's) source through `@shell`
    # and `@companion`, so their packages are counted too: a notice too many is harmless, one missing isn't.
    ("companion", "r1os Companion for Windows", ("r1os-companion", "x86_64-pc-windows-msvc"), ["companion", "shell"]),
    ("shell", "the shell's pages and the app runtime, served by r1d", None, ["shell"]),
    ("web", "the website", None, ["web", "shell", "companion"]),
]

# A licence text in a package's top folder.
TEXT = re.compile(r"^(licen[cs]e|copying|notice|unlicense|copyright)([-_.].*)?$", re.I)
# Worth a look before shipping: obligations beyond keeping the notice.
COPYLEFT = re.compile(r"\b(A?GPL|LGPL|MPL|EPL|CDDL|CC-BY-SA|EUPL|OSL|SSPL)", re.I)


def run(cmd):
    exe = shutil.which(cmd[0]) or cmd[0]
    env = dict(os.environ, RUSTFLAGS="")  # a global +crt-static mustn't change what cargo resolves
    r = subprocess.run([exe, *cmd[1:]], cwd=ROOT, env=env, capture_output=True, text=True, encoding="utf-8", errors="replace")
    if r.returncode != 0:
        sys.exit(f"{' '.join(cmd)} failed:\n{r.stderr.strip()}")
    return r.stdout


def texts(folder):
    """The licence texts in a package's folder: [(file name, text)]."""
    try:
        names = sorted(n for n in os.listdir(folder) if TEXT.match(n) and os.path.isfile(os.path.join(folder, n)))
    except OSError:
        return []
    out = []
    for n in names:
        with open(os.path.join(folder, n), encoding="utf-8", errors="replace") as f:
            out.append((n, f.read().strip()))
    return out


def rust(package, target, dirs):
    """[(name, version, licence, folder)] for [package]'s normal dependencies on [target], without r1os's own."""
    tree = run(["cargo", "tree", "-p", package, "--target", target, "-e", "normal", "--prefix", "none", "--format", "{p}|{l}"])
    seen = {}
    for line in tree.splitlines():
        line = line.replace(" (*)", "").strip()
        if not line or "|" not in line:
            continue
        p, licence = line.split("|", 1)
        if "(proc-macro)" in p or re.search(r"\((?![a-z+]+\+)[A-Za-z]:\\|\(/", p):
            continue  # not linked in; or a path dependency: r1os's own
        m = re.match(r"(\S+) v(\S+)", p)
        if m:
            name, version = m.groups()
            seen[(name, version)] = (licence.strip() or "unknown", dirs.get((name, version)))
    return [(n, v, l, d) for (n, v), (l, d) in sorted(seen.items())]


def npm(folder):
    """[(name, version, licence, folder)] for an npm package's production dependencies (with its workspace packages'),
    without r1os's own. `--dir` would list the whole workspace's."""
    with open(os.path.join(ROOT, folder, "package.json"), encoding="utf-8") as f:
        name = json.load(f)["name"]
    found = json.loads(run(["pnpm", "--filter", f"{name}...", "licenses", "list", "--prod", "--json"]) or "{}")
    out = {}
    for licence, pkgs in found.items():
        for p in pkgs:
            for version, path in zip(p["versions"], p["paths"]):
                if "node_modules" in path:
                    out[(p["name"], version)] = (licence, path)
    return [(n, v, l, d) for (n, v), (l, d) in sorted(out.items())]


def unique(pkgs):
    """[pkgs] without repeats, in order."""
    return sorted({(n, v): (n, v, l, d) for n, v, l, d in pkgs}.values())


def write(name, what, pkgs, today):
    by_text = {}  # text → [(package, file)]
    missing = []
    for n, v, licence, folder in pkgs:
        found = texts(folder) if folder else []
        if not found:
            missing.append((n, v, licence))
        for file, text in found:
            by_text.setdefault(text, []).append((f"{n} {v}", file))
    lines = [
        f"Third-party notices: {what}",
        f"Generated by tools/notices.py on {today}, {len(pkgs)} packages. r1os's own code is under the MIT licence;",
        "NOTICE.md in r1os's source covers what isn't a package (fonts, models, AOSP).",
        "",
        "Packages",
        "--------",
        *(f"{n} {v}  {licence}" for n, v, licence, _ in pkgs),
        "",
        "Licence texts",
        "-------------",
    ]
    for text, holders in sorted(by_text.items(), key=lambda kv: kv[1][0]):
        lines += ["", "---- " + ", ".join(f"{p} ({f})" for p, f in holders) + " ----", "", text]
    if missing:
        lines += ["", "No licence text in the package (its licence is as named above)", "-" * 63]
        lines += [f"{n} {v}  {licence}" for n, v, licence in missing]
    os.makedirs(OUT, exist_ok=True)
    path = os.path.join(OUT, f"{name}.txt")
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(lines) + "\n")
    return path, missing


def main():
    today = datetime.date.today().isoformat()
    meta = json.loads(run(["cargo", "metadata", "--format-version", "1", "--locked"]))
    dirs = {(p["name"], p["version"]): os.path.dirname(p["manifest_path"]) for p in meta["packages"]}
    for name, what, crate, folders in TARGETS:
        pkgs = rust(*crate, dirs) if crate else []
        for folder in folders:
            pkgs += npm(folder)
        pkgs = unique(pkgs)
        path, missing = write(name, what, pkgs, today)
        copyleft = sorted({f"{n} {v} ({l})" for n, v, l, _ in pkgs if COPYLEFT.search(l) and not re.search(r"\bOR\b", l)})
        either = sorted({f"{n} {v} ({l})" for n, v, l, _ in pkgs if COPYLEFT.search(l) and re.search(r"\bOR\b", l)})
        print(f"{os.path.relpath(path, ROOT)}: {len(pkgs)} packages, {len(missing)} without a licence text")
        for p in copyleft:
            print(f"  copyleft: {p}")
        for p in either:
            print(f"  copyleft or not, a choice: {p}")
        for n, v, l in missing:
            print(f"  no text: {n} {v} ({l})")


if __name__ == "__main__":
    main()
