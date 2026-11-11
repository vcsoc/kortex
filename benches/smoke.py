#!/usr/bin/env python3
"""Deterministic small end-to-end benchmark, not the specification's scale gate."""
import argparse
import json
import os
import pathlib
import platform
import statistics
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser()
parser.add_argument("--binary", default="target/release/kortex")
parser.add_argument("--output", default="docs/benchmark-local.json")
args = parser.parse_args()
binary = pathlib.Path(args.binary).resolve()
results = {"platform": platform.platform(), "machine": platform.machine(),
           "python": platform.python_version(), "rust": subprocess.check_output(["rustc", "--version"], text=True).strip(),
           "dataset": {"language": "python", "files": 200, "functions_per_file": 5},
           "scope": "smoke only; not 100k-symbol/10k-file acceptance, cold cache or correctness evaluation"}
with tempfile.TemporaryDirectory(prefix="kortex-bench-") as temporary:
    root = pathlib.Path(temporary)
    environment = dict(os.environ, KORTEX_REGISTRY=str(root / "registry.db"))
    for i in range(200):
        source = ""
        for j in range(5):
            body = "pass" if j == 0 else f"f_{i}_{j-1}()"
            source += f"def f_{i}_{j}():\n    {body}\n"
        (root / f"module_{i}.py").write_text(source)

    def run(*arguments):
        start = time.perf_counter()
        output = subprocess.check_output([str(binary), *arguments, "--json"], cwd=root, env=environment)
        elapsed = (time.perf_counter() - start) * 1000
        return json.loads(output), elapsed

    def summarize(samples):
        ordered = sorted(samples)
        return {"runs": len(samples), "median_ms": statistics.median(samples),
                "p95_ms": ordered[max(0, (len(ordered) * 95 + 99) // 100 - 1)]}

    results["version_warm"] = summarize([run("version")[1] for _ in range(100)])
    inventory, elapsed = run("index", ".", "--full", "--verify")
    results["full_index"] = {"elapsed_ms": elapsed, "files": inventory["files"], "symbols": inventory["symbols"], "edges": inventory["edges"]}
    results["query_warm"] = summarize([run("query", f"f_{i}_2")[1] for i in range(20)])
    results["context_warm"] = summarize([run("context", f"f_{i}_2")[1] for i in range(20)])
    results["impact_warm"] = summarize([run("impact", f"f_{i}_0")[1] for i in range(20)])
    with (root / "module_0.py").open("a") as source:
        source.write("\ndef newly_added():\n    f_0_4()\n")
    update, elapsed = run("index", ".", "--incremental", "--verify")
    results["single_file_update"] = {"elapsed_ms": elapsed, "parsed": update["parsed"], "reused": update["reused"]}
output = pathlib.Path(args.output)
output.parent.mkdir(parents=True, exist_ok=True)
output.write_text(json.dumps(results, indent=2) + "\n")
print(json.dumps(results, indent=2))
