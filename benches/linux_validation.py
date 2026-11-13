#!/usr/bin/env python3
"""Bounded, offline, local ARM64 Linux validation of an explicitly pinned snapshot.

No live checkout capture, image pull, remote engine, source upload or release.
Native Docker cache volumes are private, labeled, retained and never removed.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parent.parent
IMAGE = "rust@sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c"
EXCLUDE = {"target", ".git", ".kortex", "node_modules", "__pycache__"}
LABEL = "io.kortex.local-linux-validation.owner"
JOURNEY = "actual_cli_discovery_query_and_context_for_all_six_p1_languages"
ANALYSIS_PROOF = "read_query_tests::actual_cli_query_uses_lazy_dispatch_but_context_keeps_honest_eager_fallback"


def source_inputs(root):
    paths = [root / name for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml",
                                      "LICENSE", "README.md", "KORTEX_SPECIFICATION.md")]
    for folder in ("crates", "fixtures"):
        if (root / folder).is_symlink():
            raise ValueError("source directory aliases require explicit review")
        for directory, children, files in os.walk(root / folder, followlinks=False):
            children[:] = sorted(name for name in children if name not in EXCLUDE)
            if any((Path(directory) / name).is_symlink() for name in children):
                raise ValueError("source directory aliases require explicit review")
            paths.extend(Path(directory) / name for name in files if name != ".DS_Store")
    data, total = {}, 0
    for path in sorted(paths):
        if not stat.S_ISREG(path.lstat().st_mode):
            raise ValueError("source snapshot refuses symlinks and nonregular files")
        if len(data) >= 10000 or path.stat().st_size > 16 * 1024 * 1024:
            raise ValueError("source snapshot resource policy exceeded")
        with path.open("rb") as stream:
            content = stream.read(16 * 1024 * 1024 + 1)
        if len(content) > 16 * 1024 * 1024:
            raise ValueError("source grew beyond the snapshot file budget")
        total += len(content)
        if total > 128 * 1024 * 1024:
            raise ValueError("source snapshot total-byte policy exceeded")
        data[path.relative_to(root).as_posix()] = content
    return data


def pinned_snapshot(root, expected, count):
    if not re.fullmatch(r"[0-9a-f]{64}", expected):
        raise ValueError("expected source SHA256 must be 64 lowercase hex digits")
    root = root.absolute()
    if root.is_symlink() or root.resolve() != root:
        raise ValueError("snapshot path must not contain aliases")
    inputs = source_inputs(root)
    digest = hashlib.sha256()
    for name, content in sorted(inputs.items()):
        digest.update(name.encode() + b"\0" + content + b"\0")
    if digest.hexdigest() != expected or len(inputs) != count:
        raise ValueError("frozen snapshot hash or file count differs from explicit pin")
    return root, inputs


def local_endpoint(context):
    endpoint = context["Endpoints"]["docker"]["Host"]
    if not endpoint.startswith("unix:///") or "?" in endpoint or "#" in endpoint:
        raise ValueError("only a local absolute Unix socket endpoint is permitted")
    if not stat.S_ISSOCK(os.stat(endpoint[7:]).st_mode):
        raise ValueError("Docker endpoint is not a local Unix socket")
    return endpoint


def phase(text, name):
    match = re.search(rf"^VALIDATION_PHASE={name} BEGIN\n(.*?)^VALIDATION_PHASE={name} EXIT=(\d+)$",
                      text, re.M | re.S)
    return (match.group(1), int(match.group(2))) if match else ("", None)


def test_evidence(text):
    workspace, code = phase(text, "workspace")
    # Subprocess crash-helper summaries are nested inside a target; count its final summary once.
    summaries = []
    for block in re.split(r"(?m)^\s*(?:Running|Doc-tests) ", workspace):
        results = re.findall(r"test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;", block)
        if results:
            summaries.append(tuple(map(int, results[-1])))
    journey, journey_code = phase(text, "journey")
    required = all(f"P1_CLI language={language} discovered=3 persisted=true query_check=true context_exact_callers=1"
                   in journey for language in ("java", "csharp", "c", "cpp", "ruby", "php"))
    actual = (journey_code == 0 and required and f"test {JOURNEY} ... ok" in journey
              and "test result: ok. 1 passed; 0 failed;" in journey)
    analysis, analysis_code = phase(text, "analysis-proof")
    analysis_actual = (analysis_code == 0 and f"test {ANALYSIS_PROOF} ... ok" in analysis
                       and "test result: ok. 1 passed; 0 failed;" in analysis
                       and "actual CLI BLAKE3 " in analysis)
    return {"workspace_passed": code == 0 and bool(summaries)
            and sum(p for p, _, _ in summaries) > 0
            and all(f == 0 for _, f, _ in summaries)
            and "test result: FAILED" not in workspace,
            "workspace_passed_tests": sum(p for p, _, _ in summaries),
            "workspace_failed_tests": sum(f for _, f, _ in summaries),
            "workspace_ignored_tests": sum(i for _, _, i in summaries),
            "workspace_summarized_targets": len(summaries),
            "workspace_exit_code": code, "six_language_cli_journey_passed": actual,
            "journey_exit_code": journey_code,
            "analysis_lazy_cli_proof_passed": analysis_actual,
            "analysis_proof_exit_code": analysis_code}


def require_cache_provenance(seed, state=None):
    if seed is not None:
        raise ValueError("imported Cargo targets have no byte-bound provenance; use a fresh volume")
    if state is not None and state.get("cache_provenance") != "unseeded-v1":
        raise ValueError("resume cache lacks unseeded provenance; retain it and use a fresh volume")


def mount(kind, source, guest, readonly=False, extra=""):
    if any(c in str(source) for c in (",", "\n", "\r")):
        raise ValueError("ambiguous mount source")
    return ["--mount", f"type={kind},src={source},dst={guest}"
            + (",readonly" if readonly else "") + extra]


# Keep Cargo paths, features, registry and native target stable across retries.
SCRIPT = r'''set -eu
umask 077
mkdir -p /work/cargo/registry /work/home /work/tmp
export TMPDIR=/work/tmp
printf 'VALIDATION_METADATA BEGIN\n'
date -u; id; uname -a; cat /etc/os-release; rustc -vV; cargo -vV; git --version; cc --version
stat -f -c '%T %a %S %n' /src /build /work /work/cargo/registry
stat -c '%Y %y %n' /src/Cargo.lock /src/crates/kortex-cli/src/main.rs
df -h /build /src; cat /sys/fs/cgroup/cpu.max /sys/fs/cgroup/memory.max /sys/fs/cgroup/pids.max
cat /proc/meminfo /proc/loadavg; ls /sys/class/net
printf 'VALIDATION_METADATA END\n'
set +e
printf 'VALIDATION_PHASE=workspace BEGIN\n'
CARGO_LOG=cargo::compiler::fingerprint=debug cargo test --workspace --all-features --locked --offline --no-fail-fast -j 4
workspace=$?
printf 'VALIDATION_PHASE=workspace EXIT=%s\n' "$workspace"
printf 'VALIDATION_PHASE=build BEGIN\n'
cargo build --workspace --all-features --locked --offline -j 4
build=$?
printf 'VALIDATION_PHASE=build EXIT=%s\n' "$build"
if [ "$build" -ne 0 ]; then exit "$build"; fi
printf 'VALIDATION_BINARY BEGIN\n'
sha256sum /build/debug/kortex
file /build/debug/kortex; ldd /build/debug/kortex
printf 'VALIDATION_BINARY END\n'
printf 'VALIDATION_PHASE=journey BEGIN\n'
KORTEX_TEST_CLI=/build/debug/kortex cargo test -p kortex-indexer --test p1_discovery --all-features --locked --offline -j 4 actual_cli_discovery_query_and_context_for_all_six_p1_languages -- --ignored --exact --nocapture
journey=$?
printf 'VALIDATION_PHASE=journey EXIT=%s\n' "$journey"
printf 'VALIDATION_PHASE=analysis-proof BEGIN\n'
KORTEX_TEST_CLI=/build/debug/kortex cargo test --workspace --lib --all-features --locked --offline -j 4 read_query_tests::actual_cli_query_uses_lazy_dispatch_but_context_keeps_honest_eager_fallback -- --ignored --exact --nocapture
analysis=$?
printf 'VALIDATION_PHASE=analysis-proof EXIT=%s\n' "$analysis"
if [ "$workspace" -ne 0 ] || [ "$journey" -ne 0 ] || [ "$analysis" -ne 0 ]; then exit 1; fi
printf 'VALIDATION_PHASE=install BEGIN\n'
set -e
# This is a private debug executable install/extraction smoke, not release packaging.
base=$(mktemp -d /work/install-smoke-XXXXXX)
mkdir -p "$base/install/bin" "$base/extracted" "$base/repo" "$base/home"
install -m 755 /build/debug/kortex "$base/install/bin/kortex"
tar -czf "$base/private-linux-debug.tar.gz" -C "$base/install" bin/kortex
tar -xzf "$base/private-linux-debug.tar.gz" -C "$base/extracted"
cmp /build/debug/kortex "$base/extracted/bin/kortex"
sha256sum /build/debug/kortex "$base/install/bin/kortex" "$base/extracted/bin/kortex" "$base/private-linux-debug.tar.gz"
printf 'INSTALL_BASE=%s\n' "$base"
export HOME="$base/home" XDG_CONFIG_HOME="$base/home/config" XDG_CACHE_HOME="$base/home/cache" KORTEX_REGISTRY="$base/home/registry.db"
printf 'def check():\n    return 1\n' > "$base/repo/lib.py"
printf 'from lib import check\ndef entry():\n    return check()\n' > "$base/repo/main.py"
printf 'schema_version = 1\n[index]\nstorage = "repository"\n[search]\nembeddings = "disabled"\n[http]\nenabled = false\n' > "$base/repo/.kortex.toml"
exe="$base/extracted/bin/kortex"
"$exe" version --json > /evidence/version.json
"$exe" --repo "$base/repo" index "$base/repo" --verify --json > /evidence/index.json
"$exe" --repo "$base/repo" query check --json > /evidence/query.json
"$exe" --repo "$base/repo" context lib.py:check --json > /evidence/context.json
"$exe" --repo "$base/repo" impact lib.py:check --direction upstream --depth 3 --json > /evidence/impact.json
"$exe" --repo "$base/repo" doctor --json > /evidence/doctor.json
printf 'VALIDATION_PHASE=install EXIT=0\n'
'''


def smoke_evidence(run, text):
    _, code = phase(text, "install")
    if code != 0:
        return False
    data = {name: json.loads((run / f"{name}.json").read_text())
            for name in ("version", "index", "query", "context", "impact", "doctor")}
    return (all(value.get("schema_version") == "1.0" and "error" not in value
                for value in data.values())
            and data["version"]["version"] == "0.1.0"
            and data["index"]["parsed"] == 2
            and all(data[name].get("index_status") == "fresh"
                    and not data[name].get("truncated") for name in ("query", "context", "impact", "doctor"))
            and any(item["symbol"]["name"] == "check"
                    and item["symbol"]["file_path"] == "lib.py"
                    for item in data["query"]["data"]["items"])
            and any(item.get("resolution") == "exact"
                    for item in data["context"]["data"]["callers"])
            and data["impact"]["data"]["direct"] == 1
            and any(item.get("name") == "entry" and item.get("file_path") == "main.py"
                    for item in data["impact"]["data"]["symbols"])
            and data["doctor"]["data"]["integrity"] == "ok")


def container(docker, run, owner, name, options, script, seconds, root=False):
    """Both watchdogs bound execution; forcibly remove only our named container."""
    command = docker + ["create", "--pull=never", "--name", name, "--label", f"{LABEL}={owner}",
                        "--init", "--read-only", "--network=none", "--cap-drop=ALL", "--security-opt=no-new-privileges",
                        "--cpus=4", "--memory=8g", "--memory-swap=8g", "--pids-limit=1024",
                        "--user=0:0" if root else "--user=501:20"]
    if root:
        command += ["--cap-add=CHOWN"]
    command += options + [IMAGE, "timeout", "--signal=TERM", "--kill-after=10s", f"{seconds}s", "sh", "-c", script]
    created = False
    try:
        subprocess.check_output(command, timeout=20)
        created = True
        inspection = json.loads(subprocess.check_output(docker + ["inspect", name], timeout=10))[0]
        (run / f"{name}-inspect.json").write_text(json.dumps(inspection, indent=2) + "\n")
        if inspection.get("HostConfig", {}).get("ReadonlyRootfs") is not True:
            raise ValueError("container root filesystem is not verified read-only")
        with (run / f"{name}.log").open("w") as log:
            result = subprocess.run(docker + ["start", "--attach", name], stdout=log,
                                    stderr=subprocess.STDOUT, timeout=seconds + 30)
        state = json.loads(subprocess.check_output(docker + ["inspect", name], timeout=10))[0]["State"]
        (run / f"{name}-state.json").write_text(json.dumps(state, indent=2) + "\n")
        return state["ExitCode"] if result.returncode == 0 else result.returncode
    finally:
        if created:
            subprocess.run(docker + ["rm", "--force", name], stdout=subprocess.DEVNULL, timeout=20, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--context", default="desktop-linux")
    parser.add_argument("--snapshot", type=Path, required=True)
    parser.add_argument("--expected-source-sha256", required=True)
    parser.add_argument("--expected-source-files", type=int, required=True)
    parser.add_argument("--timeout", type=int, default=1800)
    parser.add_argument("--seed-cache", type=Path, help="legacy option; imported targets are refused without byte-bound provenance")
    parser.add_argument("--resume", type=Path, help="reuse only this runner's labeled cache-state.json")
    args = parser.parse_args()
    require_cache_provenance(args.seed_cache)
    if not hasattr(os, "getuid") or (os.getuid(), os.getgid()) != (501, 20) or not 30 <= args.timeout <= 1800:
        parser.error("requires non-root host UID501:GID20 and timeout 30..1800 seconds")
    source, inputs = pinned_snapshot(args.snapshot, args.expected_source_sha256, args.expected_source_files)
    context = json.loads(subprocess.check_output(["docker", "--context", args.context, "context", "inspect", args.context], timeout=10))[0]
    endpoint = local_endpoint(context)
    if endpoint != "unix://" + str(Path.home() / ".docker/run/docker.sock"):
        parser.error("this validation permits only the owner-approved local Docker Desktop socket")
    base = ROOT / "target/local-linux-validation"
    base.mkdir(parents=True, exist_ok=True)
    if shutil.disk_usage(base).free < 8 * 1024**3:
        parser.error("at least 8 GiB host free space required")
    run = Path(tempfile.mkdtemp(prefix="native-", dir=base))
    (run / "docker-config").mkdir()
    # Pin the checked endpoint, not a subsequently mutable context; no CLI proxy/credential config.
    docker = ["docker", "--config", str(run / "docker-config"), "--host", endpoint]
    image = json.loads(subprocess.check_output(docker + ["image", "inspect", IMAGE], timeout=10))[0]
    if image["Os"] != "linux" or image["Architecture"] != "arm64":
        parser.error("already-local pinned image must be Linux arm64")
    info = json.loads(subprocess.check_output(docker + ["info", "--format", "{{json .}}"], timeout=10))
    registry = Path.home() / ".cargo/registry"
    if not registry.is_dir() or registry.is_symlink():
        parser.error("existing non-aliased offline Cargo registry required")
    owner = uuid.uuid4().hex
    new = args.resume is None
    if new:
        volume = "kortex-local-linux-" + owner
        # Do not adopt an existing volume even if Docker's create would silently reuse it.
        existing = subprocess.check_output(docker + ["volume", "ls", "--format", "{{.Name}}"], timeout=10).decode().splitlines()
        if volume in existing:
            raise ValueError("new volume unexpectedly exists")
        subprocess.check_output(docker + ["volume", "create", "--driver=local", "--label", f"{LABEL}={owner}",
                                         "--label", "io.kortex.scope=private-offline-validation", volume], timeout=10)
    else:
        state = json.loads(args.resume.read_text())
        require_cache_provenance(None, state)
        if (state["image"], state["endpoint"], state["source_sha256"]) != (IMAGE, endpoint, args.expected_source_sha256):
            raise ValueError("cache resume identity differs")
        owner, volume = state["owner"], state["volume"]
    volume_info = json.loads(subprocess.check_output(docker + ["volume", "inspect", volume], timeout=10))[0]
    if volume_info["Driver"] != "local" or volume_info.get("Options") or volume_info.get("Labels", {}).get(LABEL) != owner:
        raise ValueError("cache volume is not this runner's private local volume")
    state = {"owner": owner, "volume": volume, "image": IMAGE, "endpoint": endpoint,
             "source_sha256": args.expected_source_sha256, "cache_provenance": "unseeded-v1"}
    (run / "cache-state.json").write_text(json.dumps(state, indent=2) + "\n")
    (run / "cache-state.json").chmod(0o600)
    runner_bytes = Path(__file__).read_bytes()
    runner_snapshot = run / "executed-runner.py"
    runner_snapshot.write_bytes(runner_bytes)
    report = {**state, "source_snapshot": str(source), "source_files": len(inputs),
              "scope": "frozen-snapshot native ARM64 Linux VM functional evidence; not current live source, x86/Windows, hardware performance, power-loss or production acceptance",
              "image_id": image["Id"], "engine": {key: info[key] for key in
                  ("ServerVersion", "KernelVersion", "Architecture", "NCPU", "MemTotal", "ContainersRunning")},
              "network": "none", "uploaded": False, "source_mount": "readonly", "registry_mount": "readonly",
              "runner_path": str(Path(__file__).resolve()),
              "runner_sha256": hashlib.sha256(runner_bytes).hexdigest(),
              "runner_snapshot": str(runner_snapshot),
              "seed_cache_requested": args.seed_cache is not None,
              "resumed_cache_requested": args.resume is not None,
              "root_filesystem_readonly": None,
              "container_uid": 501, "container_gid": 20, "native_target_volume": volume,
              "timeout_seconds": args.timeout, "linux_runtime_passed": False, "private_install_passed": False,
              "evidence_directory": str(run)}
    output = run / "report.json"
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(f"Evidence: {output}", flush=True)
    started = time.monotonic()
    try:
        if new:
            bootstrap = "test -z \"$(ls -A /build)\" && chmod 700 /build && chown 501:20 /build"
            code = container(docker, run, owner, "kortex-bootstrap-" + uuid.uuid4().hex,
                             mount("volume", volume, "/build", extra=",volume-nocopy"), bootstrap, 60, root=True)
            if code:
                raise RuntimeError(f"private volume bootstrap failed: {code}")
            setup = "umask 077; mkdir -p /build/work/cargo/registry /build/work/home /build/work/tmp"
            code = container(docker, run, owner, "kortex-setup-" + uuid.uuid4().hex,
                             mount("volume", volume, "/build", extra=",volume-nocopy"), setup, 60)
            if code:
                raise RuntimeError(f"private volume setup failed: {code}")
        # Revalidate immediately before source/registry binds; daemon command stays pinned.
        local_endpoint(context)
        pinned_snapshot(source, args.expected_source_sha256, args.expected_source_files)
        options = (mount("bind", source, "/src", True) + mount("volume", volume, "/build", extra=",volume-nocopy")
                   + mount("volume", volume, "/work", extra=",volume-nocopy,volume-subpath=work")
                   + mount("bind", registry, "/work/cargo/registry", True) + mount("bind", run, "/evidence"))
        environment = {"HOME": "/work/home", "CARGO_HOME": "/work/cargo", "CARGO_TARGET_DIR": "/build",
                       "CARGO_NET_OFFLINE": "true", "CARGO_TERM_COLOR": "never", "RUSTUP_HOME": "/usr/local/rustup",
                       "RUSTUP_TOOLCHAIN": "1.99.1", "KORTEX_REGISTRY": "/work/home/registry.db",
                       "GIT_CONFIG_SYSTEM": "/dev/null", "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_TERMINAL_PROMPT": "0"}
        for key, value in environment.items():
            options += ["--env", key + "=" + value]
        options += ["--workdir=/src"]
        name = "kortex-tests-" + uuid.uuid4().hex
        report["log"] = str(run / f"{name}.log")
        report["container_name"] = name
        output.write_text(json.dumps(report, indent=2) + "\n")
        code = container(docker, run, owner, name, options, SCRIPT, args.timeout)
        report["root_filesystem_readonly"] = True
        text = Path(report["log"]).read_text(errors="replace")
        report.update(test_evidence(text))
        report["exit_code"] = code
        hashes = re.findall(r"^([0-9a-f]{64})  /build/debug/kortex$", text, re.M)
        report["binary_sha256"] = hashes[0] if hashes else None
        report["private_install_passed"] = smoke_evidence(run, text)
        report["linux_runtime_passed"] = (code == 0 and report["workspace_passed"]
                                          and report["six_language_cli_journey_passed"]
                                          and report["analysis_lazy_cli_proof_passed"]
                                          and report["private_install_passed"])
        if source_inputs(source) != inputs:
            report["linux_runtime_passed"] = False
            raise ValueError("frozen source changed during validation")
        report["source_pin_verified_after"] = True
    except Exception as error:
        report["linux_runtime_passed"] = False
        report["error"] = {"type": type(error).__name__, "message": str(error)[:2000]}
    report["elapsed_seconds"] = time.monotonic() - started
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    return 0 if report["linux_runtime_passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
