#!/usr/bin/env python3
"""Reproducible local performance fixture; never a production-readiness assertion."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import tempfile
import time


def distribution(samples):
    ordered = sorted(samples)
    return {
        "runs": len(ordered),
        "median_ms": statistics.median(ordered),
        "p95_ms": ordered[math.ceil(len(ordered) * 0.95) - 1],
        "max_ms": ordered[-1],
    }


def record_child_rss(report):
    try:
        import resource
        peak = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
        report["peak_child_rss_bytes"] = peak if platform.system() == "Darwin" else peak * 1024
        report["rss_scope"] = "maximum across all reaped harness child commands, not phase-specific"
    except (ImportError, AttributeError, OSError):
        report["peak_child_rss_bytes"] = None
        report["rss_scope"] = "unavailable on this platform; requires separate measured evidence"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/kortex"))
    parser.add_argument("--files", type=int, default=10000)
    parser.add_argument("--functions", type=int, default=10)
    parser.add_argument("--search-runs", type=int, default=1000)
    parser.add_argument("--context-runs", type=int, default=1000)
    parser.add_argument("--impact-runs", type=int, default=1000)
    parser.add_argument("--startup-runs", type=int, default=100)
    parser.add_argument("--updates", type=int, default=30)
    parser.add_argument("--command-timeout", type=float, default=300)
    parser.add_argument("--output", type=Path, default=Path("target/acceptance-local.json"))
    parser.add_argument("--enforce-targets", action="store_true")
    args = parser.parse_args()
    counts = [args.files, args.functions, args.search_runs, args.context_runs,
              args.impact_runs, args.startup_runs, args.updates]
    if min(counts) < 1 or args.command_timeout <= 0:
        parser.error("counts and timeout must be positive")
    binary = args.binary.resolve(strict=True)
    if args.files > 100000 or args.files * args.functions > 1000000:
        parser.error("fixture exceeds harness resource policy")
    report = {
        "schema_version": 1,
        "platform": platform.platform(),
        "machine": platform.machine(),
        "python": platform.python_version(),
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "dataset": {"language": "python", "files": args.files,
                    "declared_functions": args.files * args.functions},
        "scope": "local synthetic fixture; not broad language quality or production acceptance",
        "cold_cache": "not measured; no privileged cache flushing performed",
        "source_execution": False,
        "binary_source": str(binary),
        "binary_execution": "private immutable copy verified against binary_sha256",
        "requested_samples": {"query": args.search_runs, "context": args.context_runs,
                              "impact": args.impact_runs, "startup": args.startup_runs,
                              "updates": args.updates},
        "process_commands_completed": {},
        "acceptance_sample_sizes": False,
        "runs_complete": False,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)

    def save():
        args.output.write_text(json.dumps(report, indent=2) + "\n")

    save()
    try:
        with tempfile.TemporaryDirectory(prefix="kortex-acceptance-") as directory:
            base = Path(directory).resolve()
            executable = base / ("kortex.exe" if os.name == "nt" else "kortex")
            shutil.copy2(binary, executable)
            if hashlib.sha256(executable.read_bytes()).hexdigest() != report["binary_sha256"]:
                raise RuntimeError("binary changed while creating the private benchmark copy")
            binary = executable
            root = base / "repository"
            root.mkdir()
            home = base / "home"
            home.mkdir()
            env = {k: v for k, v in os.environ.items() if not k.startswith("KORTEX_")}
            env.update(HOME=str(home), USERPROFILE=str(home),
                       XDG_CONFIG_HOME=str(home / "config"),
                       XDG_CACHE_HOME=str(home / "cache"),
                       KORTEX_REGISTRY=str(base / "registry.db"),
                       GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_SYSTEM=os.devnull,
                       GIT_TERMINAL_PROMPT="0")
            (root / ".kortex.toml").write_text(
                'schema_version = 1\n[index]\nstorage = "repository"\n'
                '[search]\nembeddings = "disabled"\n[http]\nenabled = false\n'
            )
            source_hash = hashlib.sha256()
            for file_index in range(args.files):
                parts = []
                if file_index:
                    parts.append(f"from file_{file_index - 1:05d} import fn_{file_index - 1}_{args.functions - 1}\n")
                for function in range(args.functions):
                    previous = (f"fn_{file_index}_{function - 1}()" if function else
                                f"fn_{file_index - 1}_{args.functions - 1}()" if file_index else "0")
                    parts.append(f"def fn_{file_index}_{function}():\n    return {previous}\n\n")
                source = "".join(parts)
                name = f"file_{file_index:05d}.py"
                (root / name).write_text(source, encoding="utf-8", newline="\n")
                source_hash.update(name.encode() + b"\0" + source.encode() + b"\0")
            report["dataset"]["sha256"] = source_hash.hexdigest()

            def command(*words):
                command_report = {"operation": words[0], "exit_code": None}
                report["last_command"] = command_report
                started = time.perf_counter()
                try:
                    result = subprocess.run(
                        [str(binary), "--config", str(root / ".kortex.toml"), *words, "--json"],
                        cwd=root, env=env, capture_output=True, text=True,
                        timeout=args.command_timeout, check=False,
                    )
                finally:
                    elapsed = (time.perf_counter() - started) * 1000
                    command_report["elapsed_ms"] = elapsed
                command_report["exit_code"] = result.returncode
                if result.returncode:
                    raise RuntimeError(f"command {words!r} exited {result.returncode}: {result.stderr[:2000]} {result.stdout[:2000]}")
                value = json.loads(result.stdout)
                completed = report["process_commands_completed"]
                completed[words[0]] = completed.get(words[0], 0) + 1
                return value, elapsed

            startup = [command("version")[1] for _ in range(args.startup_runs)]
            report["startup_warm"] = distribution(startup)
            indexed, elapsed = command("index", str(root), "--full", "--verify")
            if indexed["files"] != args.files:
                raise AssertionError("fixture inventory was not fully indexed")
            report["full_index"] = {"elapsed_ms": elapsed, "report": indexed}
            save()
            for operation, runs in [("query", args.search_runs), ("context", args.context_runs),
                                    ("impact", args.impact_runs)]:
                durations = []
                for index in range(runs):
                    # Stable modular sampling across the entire fixture, not only its first files.
                    file_index = (index * 7919) % args.files
                    function = index % args.functions
                    name = f"fn_{file_index}_{function}"
                    words = [operation, name]
                    if operation == "impact":
                        words.extend(["--depth", "3"])
                    value, latency = command(*words)
                    if operation == "query":
                        items = value["data"]["items"]
                        if not items or items[0]["symbol"]["name"] != name:
                            raise AssertionError("exact-name ranking failed")
                    elif operation == "context":
                        if value["data"]["symbol"]["name"] != name:
                            raise AssertionError("context selected the wrong fixture symbol")
                    else:
                        if value["data"]["target"]["name"] != name:
                            raise AssertionError("impact selected the wrong fixture symbol")
                        ordinal = file_index * args.functions + function
                        if ordinal + 1 < args.files * args.functions and value["data"]["direct"] < 1:
                            raise AssertionError("impact omitted a known fixture caller")
                    durations.append(latency)
                report[f"{operation}_warm"] = distribution(durations)
                save()
            changed = root / f"file_{args.files - 1:05d}.py"
            original = changed.read_text()
            updates = []
            for update in range(args.updates):
                changed.write_text(original + f"\n# controlled benchmark update {update}\n", encoding="utf-8", newline="\n")
                value, latency = command("index", str(root), "--incremental", "--verify")
                if value["parsed"] != 1 or value["reused"] != args.files - 1:
                    raise AssertionError("single-file fact reuse invariant failed")
                updates.append(latency)
            report["single_file_update"] = distribution(updates)
            report["incremental_measurement"] = "direct index invocation to committed result; not watcher latency"
            report["acceptance_sample_sizes"] = (
                args.files >= 10000 and args.files * args.functions >= 100000
                and min(args.search_runs, args.context_runs, args.impact_runs) >= 1000
                and args.startup_runs >= 100 and args.updates >= 30
            )
            thresholds = {"startup_warm": 100, "query_warm": 150, "context_warm": 200,
                          "impact_warm": 500, "single_file_update": 2000}
            report["latency_targets"] = {
                name: {"target_p95_ms": maximum, "observed_p95_ms": report[name]["p95_ms"],
                       "passed": report[name]["p95_ms"] < maximum}
                for name, maximum in thresholds.items()
            }
            record_child_rss(report)
            report["runs_complete"] = True
            save()
            print(json.dumps(report, indent=2))
            if args.enforce_targets and (not report["acceptance_sample_sizes"] or
                                        not all(item["passed"] for item in report["latency_targets"].values())):
                return 1
    except Exception as error:
        report["error"] = {"type": type(error).__name__, "message": str(error)[:4000]}
        record_child_rss(report)
        save()
        raise
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
