#!/usr/bin/env python3
"""Build and smoke test a local EMT release candidate wheel."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import textwrap
import time
from dataclasses import asdict, dataclass
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_OUT_DIR = PROJECT_ROOT / ".artifacts" / "release-candidate"


class SmokeFailure(RuntimeError):
    """Raised when a release-candidate smoke step fails."""


@dataclass
class CommandResult:
    name: str
    command: list[str]
    returncode: int
    duration_seconds: float
    stdout_path: str
    stderr_path: str


def run_command(
    name: str,
    command: list[str],
    *,
    cwd: Path,
    output_dir: Path,
    timeout: float | None = None,
) -> CommandResult:
    output_dir.mkdir(parents=True, exist_ok=True)
    stdout_path = output_dir / f"{name}.stdout.txt"
    stderr_path = output_dir / f"{name}.stderr.txt"
    start = time.perf_counter()
    completed = subprocess.run(
        command,
        cwd=cwd,
        text=True,
        capture_output=True,
        timeout=timeout,
        check=False,
    )
    duration_seconds = time.perf_counter() - start
    stdout_path.write_text(completed.stdout, encoding="utf-8")
    stderr_path.write_text(completed.stderr, encoding="utf-8")

    result = CommandResult(
        name=name,
        command=command,
        returncode=completed.returncode,
        duration_seconds=duration_seconds,
        stdout_path=str(stdout_path),
        stderr_path=str(stderr_path),
    )
    if completed.returncode != 0:
        raise SmokeFailure(
            f"{name} failed with exit code {completed.returncode}; "
            f"see {stdout_path} and {stderr_path}"
        )
    return result


def latest_wheel(dist_dir: Path) -> Path:
    wheels = sorted(
        dist_dir.glob("emt-*.whl"),
        key=lambda path: (path.stat().st_mtime, path.name),
        reverse=True,
    )
    if not wheels:
        raise SmokeFailure(f"No EMT wheel found under {dist_dir}")
    return wheels[0]


def wheel_looks_like_release_candidate(wheel: Path) -> bool:
    return re.search(r"^emt-\d+(?:\.\d+)*rc\d+-", wheel.name) is not None


def write_energy_monitor_smoke(path: Path) -> None:
    path.write_text(
        textwrap.dedent("""
            import json
            import math
            import os
            import time

            from emt import EnergyMonitor


            def spin(duration_seconds):
                deadline = time.perf_counter() + duration_seconds
                value = 0
                while time.perf_counter() < deadline:
                    value = ((value * 1664525) + 1013904223) & 0xFFFFFFFF
                return value


            with EnergyMonitor(
                name="release-candidate-smoke",
                pid=os.getpid(),
                rate=1.0,
                startup_delay_s=0.1,
            ) as monitor:
                spin(0.25)

            total = float(monitor.total_consumed_energy)
            consumed = dict(monitor.consumed_energy)
            if not math.isfinite(total) or total < 0.0:
                raise RuntimeError(f"invalid total energy: {total!r}")
            if not isinstance(consumed, dict):
                raise RuntimeError("consumed_energy is not a mapping")

            print(
                json.dumps(
                    {
                        "total_consumed_energy": total,
                        "consumed_energy_keys": sorted(consumed),
                    },
                    sort_keys=True,
                )
            )
            """).strip() + "\n",
        encoding="utf-8",
    )


def build_release_candidate(args: argparse.Namespace) -> dict:
    out_dir = args.out_dir.resolve()
    if args.clean and out_dir.exists():
        shutil.rmtree(out_dir)
    dist_dir = out_dir / "dist"
    command_dir = out_dir / "command-output"
    venv_dir = out_dir / "venv"
    dist_dir.mkdir(parents=True, exist_ok=True)

    maturin = shutil.which("maturin")
    if maturin is None:
        raise SmokeFailure("maturin is required on PATH to build a release candidate")

    commands: list[CommandResult] = []
    commands.append(
        run_command(
            "maturin-build",
            [maturin, "build", "--release", "--out", str(dist_dir)],
            cwd=PROJECT_ROOT,
            output_dir=command_dir,
            timeout=args.build_timeout,
        )
    )

    wheel = latest_wheel(dist_dir)
    if args.require_rc_version and not wheel_looks_like_release_candidate(wheel):
        raise SmokeFailure(
            f"Built wheel does not look like a release candidate: {wheel.name}"
        )

    commands.append(
        run_command(
            "create-venv",
            [sys.executable, "-m", "venv", str(venv_dir)],
            cwd=PROJECT_ROOT,
            output_dir=command_dir,
            timeout=120,
        )
    )
    bin_dir = venv_dir / ("Scripts" if os.name == "nt" else "bin")
    python = bin_dir / ("python.exe" if os.name == "nt" else "python")
    emt = bin_dir / ("emt.exe" if os.name == "nt" else "emt")
    emt_cfgup = bin_dir / ("emt_cfgup.exe" if os.name == "nt" else "emt_cfgup")

    commands.append(
        run_command(
            "pip-install-wheel",
            [str(python), "-m", "pip", "install", str(wheel)],
            cwd=PROJECT_ROOT,
            output_dir=command_dir,
            timeout=180,
        )
    )
    commands.append(
        run_command(
            "emt-help",
            [str(emt), "--help"],
            cwd=PROJECT_ROOT,
            output_dir=command_dir,
            timeout=30,
        )
    )

    cli_json = out_dir / "emt-cli-smoke.json"
    snapshot_json = out_dir / "emt-snapshot-smoke.json"
    commands.append(
        run_command(
            "emt-json-smoke",
            [
                str(emt),
                "--json-out",
                str(cli_json),
                "--snapshot-out",
                str(snapshot_json),
                "--duration",
                str(args.duration),
                "--rate",
                "1",
                "--scan-interval",
                "1",
            ],
            cwd=PROJECT_ROOT,
            output_dir=command_dir,
            timeout=args.cli_timeout,
        )
    )

    with cli_json.open("r", encoding="utf-8") as file:
        cli_output = json.load(file)
    with snapshot_json.open("r", encoding="utf-8") as file:
        snapshot_output = json.load(file)

    smoke_script = out_dir / "energy_monitor_smoke.py"
    write_energy_monitor_smoke(smoke_script)
    commands.append(
        run_command(
            "energy-monitor-smoke",
            [str(python), str(smoke_script)],
            cwd=PROJECT_ROOT,
            output_dir=command_dir,
            timeout=args.python_timeout,
        )
    )
    commands.append(
        run_command(
            "emt-cfgup-help",
            [str(emt_cfgup), "--help"],
            cwd=PROJECT_ROOT,
            output_dir=command_dir,
            timeout=30,
        )
    )

    report = {
        "wheel": str(wheel),
        "wheel_name": wheel.name,
        "out_dir": str(out_dir),
        "venv": str(venv_dir),
        "emt_command": str(emt),
        "emt_cfgup_command": str(emt_cfgup),
        "cli_json": str(cli_json),
        "snapshot_json": str(snapshot_json),
        "cli_duration_seconds": cli_output.get("duration_seconds"),
        "cli_total_energy": cli_output.get("total_energy"),
        "snapshot_workload_count": len(snapshot_output.get("workloads", [])),
        "commands": [asdict(command) for command in commands],
    }
    report_path = out_dir / "release-candidate-smoke-report.json"
    report_path.write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return report


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_OUT_DIR)
    parser.add_argument("--duration", type=int, default=1)
    parser.add_argument("--build-timeout", type=float, default=900.0)
    parser.add_argument("--cli-timeout", type=float, default=60.0)
    parser.add_argument("--python-timeout", type=float, default=60.0)
    parser.add_argument("--no-clean", dest="clean", action="store_false")
    parser.add_argument(
        "--allow-non-rc-version", dest="require_rc_version", action="store_false"
    )
    parser.set_defaults(clean=True, require_rc_version=True)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        report = build_release_candidate(args)
    except SmokeFailure as exc:
        print(f"release candidate smoke failed: {exc}", file=sys.stderr)
        return 1

    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
