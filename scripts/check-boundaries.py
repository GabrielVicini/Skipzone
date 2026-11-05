#!/usr/bin/env python3
"""Check the dependency rules between the Skipzone workspace crates.

Run from the repository root:

    python3 scripts/check-boundaries.py
    python3 scripts/check-boundaries.py --metadata meta.json   # use saved output

The layer order is engine < iono < predict < {lab, app}. A crate may depend
only on crates in lower layers, and lab and app never depend on each other.
Crates from the target layout that do not exist yet are reported as skipped.

Uses only `cargo metadata`, so nothing is compiled and no system libraries
are needed.
"""

import argparse
import json
import subprocess
import sys

ENGINE = "skipzone"

# Workspace crates and their layer. Lower layers must not depend on higher ones.
LAYERS = {
    "skipzone": 0,
    "skipzone-iono": 1,
    "skipzone-predict": 2,
    "skipzone-lab": 3,
    "skipzone-app": 3,
}

# The only normal dependencies the engine may declare.
ENGINE_ALLOWED_DEPS = {"num-complex", "thiserror"}

GUI = {"egui", "eframe", "winit"}
NET = {"ureq"}
GUI_AND_NET = GUI | NET

# Crates that must not appear anywhere in the normal dependency tree.
# rayon is allowed in skipzone-predict: its compute, solve and antenna modules
# run data-parallel batches of independent solves. The engine and iono stay
# single-threaded, so rayon is banned there.
BANNED_TRANSITIVE = {
    "skipzone": GUI_AND_NET | {"rayon"},
    "skipzone-iono": GUI_AND_NET | {"rayon"},
    "skipzone-predict": GUI_AND_NET,
    # lab is the only crate with network access and has no GUI; the app is
    # the only crate with a GUI and makes no network calls.
    "skipzone-lab": GUI,
    "skipzone-app": NET,
}


def load_metadata(path):
    if path:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--all-features", "--locked"],
        check=True,
        stdout=subprocess.PIPE,
    ).stdout
    return json.loads(out)


def is_normal(dep):
    return any(k.get("kind") is None for k in dep.get("dep_kinds", []))


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--metadata", help="read cargo metadata JSON from this file")
    args = parser.parse_args()

    meta = load_metadata(args.metadata)
    packages = {p["id"]: p for p in meta["packages"]}
    members = {packages[i]["name"]: i for i in meta["workspace_members"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}

    def name(pkg_id):
        return packages[pkg_id]["name"]

    def normal_closure(start):
        """Every package reachable from `start` through normal dependencies."""
        seen, stack = set(), [start]
        while stack:
            for dep in nodes[stack.pop()]["deps"]:
                if is_normal(dep) and dep["pkg"] not in seen:
                    seen.add(dep["pkg"])
                    stack.append(dep["pkg"])
        return seen

    failures = []
    rows = []  # (crate, check, result)

    def record(crate, check, problems):
        rows.append((crate, check, "FAIL" if problems else "ok"))
        failures.extend(f"{crate}: {p}" for p in problems)

    unknown = sorted(set(members) - set(LAYERS))
    for crate in unknown:
        record(crate, "known layer", ["not in LAYERS in scripts/check-boundaries.py"])

    for crate in LAYERS:
        if crate not in members:
            rows.append((crate, "-", "not present, skipped"))
            continue
        pkg_id = members[crate]
        node = nodes[pkg_id]

        if crate == ENGINE:
            direct = {name(d["pkg"]) for d in node["deps"] if is_normal(d)}
            extra = sorted(direct - ENGINE_ALLOWED_DEPS)
            record(crate, "direct deps only " + ", ".join(sorted(ENGINE_ALLOWED_DEPS)),
                   [f"normal dependency `{d}` is not allowed" for d in extra])

        banned = BANNED_TRANSITIVE.get(crate)
        if banned:
            found = sorted({name(p) for p in normal_closure(pkg_id)} & banned)
            record(crate, "no " + ", ".join(sorted(banned)) + " (transitive)",
                   [f"pulls in `{d}`" for d in found])

        # Edges to other workspace crates, of any kind (normal, dev, build).
        problems = []
        layer = LAYERS[crate]
        for dep in node["deps"]:
            dep_name = name(dep["pkg"])
            if dep_name not in members or dep_name not in LAYERS:
                continue
            if dep_name == "skipzone-app":
                problems.append("depends on skipzone-app")
            elif {crate, dep_name} == {"skipzone-lab", "skipzone-app"}:
                problems.append(f"depends on {dep_name} (lab and app are independent)")
            elif LAYERS[dep_name] > layer:
                problems.append(f"depends on higher-layer crate {dep_name}")
        record(crate, "workspace deps point down only", problems)

    width = max(len(r[0]) for r in rows)
    check_width = max(len(r[1]) for r in rows)
    print(f"{'crate':<{width}}  {'check':<{check_width}}  result")
    print(f"{'-' * width}  {'-' * check_width}  ------")
    for crate, check, result in rows:
        print(f"{crate:<{width}}  {check:<{check_width}}  {result}")

    if failures:
        print("\nBoundary violations:", file=sys.stderr)
        for f in failures:
            print(f"  {f}", file=sys.stderr)
        return 1
    print("\nAll dependency boundaries hold.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
