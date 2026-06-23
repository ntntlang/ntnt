#!/usr/bin/env python3
"""DD-061 performance benchmark harness for ntnt.

The harness is intentionally opt-in. It builds the dev-release binary, starts the
representative benchmark server, runs HTTP route benchmarks with `wrk` when
available (or a deterministic sequential urllib fallback), records an
interpreter-only CLI timing, and writes JSON plus Markdown summaries.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import signal
import statistics
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from urllib.error import URLError
from urllib.request import urlopen

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_SERVER = REPO_ROOT / "examples" / "perf" / "server.tnt"
DEFAULT_CLI = REPO_ROOT / "examples" / "perf" / "compute_cli.tnt"
DEFAULT_OUTPUT_DIR = REPO_ROOT / "target" / "perf-bench"
HTTP_BENCHMARKS = [
    {"name": "plaintext route", "path": "/"},
    {"name": "small JSON route", "path": "/json"},
    {"name": "route param + map read", "path": "/param/12345"},
    {"name": "compute loop route", "path": "/compute"},
    {"name": "template layout + partial", "path": "/template/layout"},
    {"name": "template 100-row loop", "path": "/template/rows"},
]
DB_BENCHMARKS = [
    {"name": "PostgreSQL single query", "path": "/db/single"},
    {"name": "PostgreSQL multi query", "path": "/db/multi"},
]


@dataclass
class CommandResult:
    command: list[str]
    returncode: int
    stdout: str
    stderr: str
    elapsed_ms: float


def run_command(command: list[str], *, cwd: Path = REPO_ROOT, env: dict[str, str] | None = None) -> CommandResult:
    started = time.perf_counter()
    proc = subprocess.run(command, cwd=cwd, env=env, text=True, capture_output=True)
    elapsed_ms = (time.perf_counter() - started) * 1000
    return CommandResult(command, proc.returncode, proc.stdout, proc.stderr, elapsed_ms)


def require_success(result: CommandResult) -> None:
    if result.returncode == 0:
        return
    command = " ".join(result.command)
    sys.stderr.write(f"Command failed: {command}\n")
    if result.stdout:
        sys.stderr.write(result.stdout)
    if result.stderr:
        sys.stderr.write(result.stderr)
    raise SystemExit(result.returncode)


def ntnt_binary(args: argparse.Namespace) -> Path:
    if args.ntnt_bin:
        return Path(args.ntnt_bin).resolve()
    return REPO_ROOT / "target" / "dev-release" / "ntnt"


def build_if_needed(args: argparse.Namespace) -> None:
    if args.skip_build:
        return
    result = run_command(["cargo", "build", "--profile", "dev-release"])
    require_success(result)


def wait_for_server(base_url: str, timeout_s: float) -> None:
    deadline = time.monotonic() + timeout_s
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            with urlopen(base_url + "/", timeout=1) as response:
                if 200 <= response.status < 500:
                    return
        except Exception as exc:  # noqa: BLE001 - readiness retry reports last error
            last_error = exc
        time.sleep(0.1)
    raise RuntimeError(f"server did not become ready at {base_url}: {last_error}")


def start_server(args: argparse.Namespace, env: dict[str, str]) -> tuple[subprocess.Popen[str], Any]:
    command = [str(ntnt_binary(args)), "run", str(DEFAULT_SERVER)]
    log_file = tempfile.TemporaryFile(mode="w+")
    proc = subprocess.Popen(
        command,
        cwd=REPO_ROOT,
        env=env,
        text=True,
        stdout=log_file,
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )
    try:
        wait_for_server(args.base_url, args.startup_timeout)
    except Exception as exc:
        stop_process(proc)
        log_file.seek(0)
        output = log_file.read()[-4000:]
        log_file.close()
        raise RuntimeError(f"failed to start benchmark server. Recent output:\n{output}") from exc
    return proc, log_file


def stop_process(proc: subprocess.Popen[str]) -> None:
    if proc.poll() is not None:
        return
    try:
        os.killpg(proc.pid, signal.SIGTERM)
        proc.wait(timeout=5)
    except Exception:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except Exception:
            pass
        try:
            proc.wait(timeout=5)
        except Exception:
            pass


def parse_wrk_output(output: str) -> dict[str, Any]:
    requests_match = re.search(r"Requests/sec:\s+([0-9.]+)", output)
    transfer_match = re.search(r"Transfer/sec:\s+([^\n]+)", output)
    latency_match = re.search(r"Latency\s+([0-9.]+)(us|ms|s)", output)
    result: dict[str, Any] = {"raw": output}
    warnings: list[str] = []
    if requests_match:
        result["requests_per_second"] = float(requests_match.group(1))
    else:
        warnings.append("wrk Requests/sec output did not match expected format")
    if transfer_match:
        result["transfer_per_second"] = transfer_match.group(1).strip()
    if latency_match:
        value = float(latency_match.group(1))
        unit = latency_match.group(2)
        multiplier = {"us": 0.001, "ms": 1.0, "s": 1000.0}[unit]
        result["avg_latency_ms"] = value * multiplier
    else:
        warnings.append("wrk Latency output did not match expected format")
    if warnings:
        result["parse_warning"] = "; ".join(warnings)
    return result


def run_wrk(url: str, args: argparse.Namespace) -> dict[str, Any]:
    command = [
        "wrk",
        "-t",
        str(args.threads),
        "-c",
        str(args.connections),
        "-d",
        args.duration,
        url,
    ]
    result = run_command(command)
    if result.returncode != 0:
        return {
            "tool": "wrk",
            "ok": False,
            "command": command,
            "error": result.stderr or result.stdout,
            "elapsed_ms": result.elapsed_ms,
        }
    parsed = parse_wrk_output(result.stdout)
    parsed.update({"tool": "wrk", "ok": True, "command": command, "elapsed_ms": result.elapsed_ms})
    return parsed


def run_urllib_loop(url: str, args: argparse.Namespace) -> dict[str, Any]:
    duration_s = parse_duration_seconds(args.duration)
    started_all = time.perf_counter()
    deadline = started_all + duration_s
    latencies_ms: list[float] = []
    errors = 0
    while time.perf_counter() < deadline:
        started = time.perf_counter()
        try:
            with urlopen(url, timeout=args.request_timeout) as response:
                response.read()
        except (OSError, URLError):
            errors += 1
            continue
        latencies_ms.append((time.perf_counter() - started) * 1000)
    elapsed_s = max(time.perf_counter() - started_all, 0.001)
    requests = len(latencies_ms)
    return {
        "tool": "urllib-sequential",
        "ok": errors == 0,
        "requests": requests,
        "errors": errors,
        "requests_per_second": requests / elapsed_s,
        "avg_latency_ms": statistics.mean(latencies_ms) if latencies_ms else None,
        "p95_latency_ms": percentile(latencies_ms, 95) if latencies_ms else None,
    }


def parse_duration_seconds(value: str) -> float:
    match = re.fullmatch(r"([0-9]+(?:\.[0-9]+)?)(s|m)?", value)
    if not match:
        raise ValueError(f"unsupported duration: {value}; use seconds or minutes, e.g. 2s or 1m")
    number = float(match.group(1))
    unit = match.group(2) or "s"
    return number * {"s": 1.0, "m": 60.0}[unit]


def percentile(values: list[float], pct: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, round((pct / 100.0) * (len(ordered) - 1))))
    return ordered[index]


def run_http_benchmark(bench: dict[str, str], args: argparse.Namespace, use_wrk: bool) -> dict[str, Any]:
    url = args.base_url + bench["path"]
    result = run_wrk(url, args) if use_wrk else run_urllib_loop(url, args)
    return {"name": bench["name"], "path": bench["path"], "url": url, **result}


def run_cli_benchmark(args: argparse.Namespace, env: dict[str, str]) -> dict[str, Any]:
    command = [str(ntnt_binary(args)), "run", str(DEFAULT_CLI)]
    timings: list[float] = []
    outputs: list[str] = []
    for _ in range(args.runs):
        result = run_command(command, env=env)
        if result.returncode != 0:
            return {
                "name": "interpreter compute CLI",
                "ok": False,
                "command": command,
                "runs": len(timings),
                "error": result.stderr or result.stdout or "CLI benchmark failed",
            }
        timings.append(result.elapsed_ms)
        outputs.append(result.stdout.strip())
    return {
        "name": "interpreter compute CLI",
        "ok": True,
        "command": command,
        "runs": args.runs,
        "median_ms": statistics.median(timings),
        "best_ms": min(timings),
        "worst_ms": max(timings),
        "outputs": outputs[-1:],
    }


def write_results(results: dict[str, Any], output_dir: Path) -> tuple[Path, Path]:
    output_dir.mkdir(parents=True, exist_ok=True)
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    json_path = output_dir / f"ntnt-perf-{stamp}.json"
    md_path = output_dir / f"ntnt-perf-{stamp}.md"
    json_path.write_text(json.dumps(results, indent=2) + "\n")
    md_path.write_text(render_markdown(results))
    return json_path, md_path


def render_markdown(results: dict[str, Any]) -> str:
    lines = [
        "# ntnt DD-061 benchmark run",
        "",
        f"- timestamp: `{results['timestamp']}`",
        f"- git: `{results['git']['short_sha']}` on `{results['git']['branch']}`",
        f"- ntnt: `{results['ntnt_binary']}`",
        f"- HTTP tool: `{results['http_tool']}`",
        "",
        "## HTTP benchmarks",
        "",
        "| Benchmark | Path | RPS | Avg latency ms | Notes |",
        "|---|---:|---:|---:|---|",
    ]
    for item in results["http"]:
        rps = format_number(item.get("requests_per_second"))
        latency = format_number(item.get("avg_latency_ms"))
        if item.get("parse_warning"):
            notes = item["parse_warning"]
        elif item.get("ok"):
            notes = "ok"
        elif "errors" in item:
            notes = f"{item['errors']} request error(s)"
        else:
            notes = item.get("error", "error")
        lines.append(f"| {item['name']} | `{item['path']}` | {rps} | {latency} | {notes} |")
    cli = results["cli"]
    lines.extend(["", "## CLI benchmark", ""])
    if cli.get("ok", False):
        lines.append(
            f"- {cli['name']}: median `{cli['median_ms']:.2f}ms`, best `{cli['best_ms']:.2f}ms`, worst `{cli['worst_ms']:.2f}ms` over {cli['runs']} run(s)"
        )
    else:
        lines.append(f"- {cli['name']}: failed after {cli['runs']} successful run(s): `{cli.get('error', 'unknown error')}`")
    lines.append("")
    return "\n".join(lines)


def format_number(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, float):
        return f"{value:.2f}"
    return str(value)


def git_context() -> dict[str, str]:
    branch = git_output(["git", "branch", "--show-current"])
    sha = git_output(["git", "rev-parse", "HEAD"])
    short_sha = sha[:12] if sha != "unknown" else "unknown"
    return {"branch": branch, "sha": sha, "short_sha": short_sha}


def git_output(command: list[str]) -> str:
    result = run_command(command)
    value = result.stdout.strip()
    if result.returncode != 0 or not value:
        return "unknown"
    return value


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run ntnt DD-061 performance benchmarks.")
    parser.add_argument("--quick", action="store_true", help="Use short durations and one CLI timing run.")
    parser.add_argument("--skip-build", action="store_true", help="Do not run cargo build before benchmarks.")
    parser.add_argument("--ntnt-bin", help="Path to ntnt binary. Defaults to target/dev-release/ntnt.")
    parser.add_argument(
        "--base-url",
        default="http://127.0.0.1:18080",
        help="Benchmark server base URL. Must match examples/perf/server.tnt's listen() port.",
    )
    parser.add_argument("--duration", default=None, help="HTTP benchmark duration per route, e.g. 3s or 10s.")
    parser.add_argument("--threads", type=int, default=2, help="wrk thread count.")
    parser.add_argument("--connections", type=int, default=16, help="wrk connection count.")
    parser.add_argument("--runs", type=int, default=None, help="CLI benchmark repetitions.")
    parser.add_argument("--include-db", action="store_true", help="Include PostgreSQL routes when DATABASE_URL is set.")
    parser.add_argument("--output-dir", type=Path, default=DEFAULT_OUTPUT_DIR)
    parser.add_argument("--startup-timeout", type=float, default=10.0)
    parser.add_argument("--request-timeout", type=float, default=5.0)
    args = parser.parse_args()
    if args.quick:
        args.duration = args.duration or "2s"
        args.runs = args.runs or 1
        args.connections = min(args.connections, 8)
    else:
        args.duration = args.duration or "10s"
        args.runs = args.runs or 3
    try:
        parse_duration_seconds(args.duration)
    except ValueError as exc:
        parser.error(str(exc))
    if re.fullmatch(r"[0-9]+(?:\.[0-9]+)?", args.duration):
        args.duration = f"{args.duration}s"
    return args


def main() -> None:
    args = parse_args()
    build_if_needed(args)

    env = os.environ.copy()
    env.setdefault("NTNT_ENV", "production")
    env.setdefault("NTNT_TYPE_MODE", "strict")
    env.setdefault("NTNT_LINT_MODE", "strict")

    use_wrk = shutil.which("wrk") is not None
    benches = list(HTTP_BENCHMARKS)
    if args.include_db and env.get("DATABASE_URL"):
        benches.extend(DB_BENCHMARKS)

    server, server_log = start_server(args, env)
    try:
        http_results = [run_http_benchmark(bench, args, use_wrk) for bench in benches]
    finally:
        stop_process(server)
        server_log.close()

    cli_result = run_cli_benchmark(args, env)
    results = {
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "git": git_context(),
        "ntnt_binary": str(ntnt_binary(args)),
        "http_tool": "wrk" if use_wrk else "urllib-sequential",
        "quick": args.quick,
        "http": http_results,
        "cli": cli_result,
        "db_included": args.include_db and bool(env.get("DATABASE_URL")),
    }
    json_path, md_path = write_results(results, args.output_dir)
    print(f"Wrote {json_path}")
    print(f"Wrote {md_path}")


if __name__ == "__main__":
    main()
