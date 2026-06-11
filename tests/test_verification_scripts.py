import builtins
import json
import subprocess
from pathlib import Path
from types import SimpleNamespace

import pytest

from scripts import probe_prometheus_power_cadence as cadence_probe
from scripts import verification_workload
from scripts import verify


def test_deterministic_value_is_stable_and_varies_by_coordinates():
    value = verification_workload.deterministic_value(2, 3, 4)

    assert value == pytest.approx(0.165)
    assert verification_workload.deterministic_value(2, 3, 4) == value
    assert verification_workload.deterministic_value(2, 3, 5) != value


def test_cpu_intensive_work_uses_deterministic_matrix_and_checksum(monkeypatch, capsys):
    times = iter([0.0, 0.0, 0.5, 2.0, 2.0])

    def fake_range(size):
        if size == 200:
            return builtins.range(2)
        return builtins.range(size)

    monkeypatch.setattr(verification_workload.time, "perf_counter", lambda: next(times))
    monkeypatch.setattr(verification_workload, "range", fake_range, raising=False)

    iterations = verification_workload.cpu_intensive_work(duration_seconds=1.0)

    expected_a = [
        [
            verification_workload.deterministic_value(row, col, 0)
            for col in builtins.range(2)
        ]
        for row in builtins.range(2)
    ]
    expected_b = [
        [
            verification_workload.deterministic_value(row, col, 1)
            for col in builtins.range(2)
        ]
        for row in builtins.range(2)
    ]
    expected_checksum = sum(
        sum(
            sum(expected_a[row][k] * expected_b[k][col] for k in builtins.range(2))
            for col in builtins.range(2)
        )
        for row in builtins.range(2)
    )

    assert iterations == 1
    assert f"checksum {expected_checksum:.2f}" in capsys.readouterr().out


def test_build_verification_methods_forwards_sudo_to_rust(monkeypatch):
    calls = []

    def fake_measure_rust_cli(duration, iteration, use_sudo=False):
        calls.append((duration, iteration, use_sudo))
        return verify.Result("rust_cli", iteration, duration, 1.0)

    monkeypatch.setattr(verify, "measure_rust_cli", fake_measure_rust_cli)

    methods = verify.build_verification_methods(use_sudo=True)
    rust_result = methods[1][1](3.5, 2)

    assert [name for name, _ in methods] == ["Python EMT", "Rust CLI"]
    assert rust_result.total_energy_j == pytest.approx(1.0)
    assert calls == [(3.5, 2, True)]


def test_measure_rust_cli_writes_output_under_artifacts_tmp(tmp_path, monkeypatch):
    rust_binary = tmp_path / "emt"
    rust_binary.write_text("#!/bin/sh\n", encoding="utf-8")
    output_dir = tmp_path / "rust-output"
    popen_calls = []

    class FakeProcess:
        def __init__(self, cmd, **kwargs):
            self.cmd = cmd
            self.kwargs = kwargs
            self.pid = 12345
            self.returncode = 0
            popen_calls.append((cmd, kwargs))

            if str(rust_binary) in cmd:
                output_file = Path(cmd[cmd.index("--json-out") + 1])
                output_file.write_text(
                    json.dumps(
                        {
                            "total_energy": 42.5,
                            "energy_unit": "Joules",
                            "devices": {"cpu": 42.5},
                        }
                    ),
                    encoding="utf-8",
                )

        def wait(self):
            return 0

        def communicate(self, timeout=None):
            return "", ""

    monkeypatch.setattr(verify, "RUST_BINARY", rust_binary)
    monkeypatch.setattr(verify, "RUST_VERIFY_TMP_DIR", output_dir)
    monkeypatch.setattr(verify.subprocess, "Popen", FakeProcess)
    monkeypatch.setattr(verify.time, "sleep", lambda _seconds: None)
    monkeypatch.setattr(verify.time, "perf_counter", iter([10.0, 14.0]).__next__)

    result = verify.measure_rust_cli(workload_duration=5.0, iteration=7)

    assert result.total_energy_j == pytest.approx(42.5)
    assert result.details["devices"] == {"cpu": 42.5}
    assert output_dir.is_dir()
    assert list(output_dir.iterdir()) == []
    rust_cmd, rust_kwargs = popen_calls[1]
    assert rust_cmd[rust_cmd.index("--json-out") + 1].startswith(str(output_dir))
    assert rust_kwargs["env"]["EMT_DISABLE_GPU"] == "1"


def test_rust_output_parser_normalizes_configured_energy_units():
    data = {
        "total_energy": 0.001,
        "energy_unit": "kWh",
        "devices": {"cpu": 0.00075, "dram": 0.00025},
    }

    assert verify.rust_total_energy_joules(data) == pytest.approx(3600.0)
    assert verify.rust_devices_joules(data) == {
        "cpu": pytest.approx(2700.0),
        "dram": pytest.approx(900.0),
    }


def test_rust_output_parser_keeps_legacy_total_energy_joules_fallback():
    assert verify.rust_total_energy_joules(
        {"total_energy_joules": 12.5}
    ) == pytest.approx(12.5)


def test_measure_method_keeps_successes_and_reports_expected_errors(
    monkeypatch, capsys
):
    sleeps = []

    def fake_measure(duration, iteration):
        if iteration == 2:
            raise ValueError("bad sample")
        return verify.Result("fake", iteration, duration + iteration, 10.0 + iteration)

    monkeypatch.setattr(verify.time, "sleep", sleeps.append)
    monkeypatch.setattr(verify, "SETTLE_SECONDS", 0.25)

    results = verify.measure_method("Fake", fake_measure, 2, 3.0)

    assert [result.iteration for result in results] == [1]
    assert results[0].duration == pytest.approx(4.0)
    assert sleeps == [1, 1, 0.25]
    output = capsys.readouterr().out
    assert "Phase: Fake" in output
    assert "bad sample" in output


def test_run_methods_invokes_each_named_method(monkeypatch):
    calls = []

    def method(name):
        def fake(duration, iteration):
            calls.append((name, duration, iteration))
            return verify.Result(name, iteration, duration, float(iteration))

        return fake

    monkeypatch.setattr(verify.time, "sleep", lambda _seconds: None)

    results = verify.run_methods(
        [("A", method("A")), ("B", method("B"))],
        num_iterations=3,
        workload_duration=2.0,
    )

    assert calls == [
        ("A", 2.0, 1),
        ("B", 2.0, 1),
        ("B", 2.0, 2),
        ("A", 2.0, 2),
        ("A", 2.0, 3),
        ("B", 2.0, 3),
    ]
    assert [result.iteration for result in results["A"]] == [1, 2, 3]
    assert [result.iteration for result in results["B"]] == [1, 2, 3]


def test_printing_results_builds_statistics_tables_and_acceptance(capsys):
    all_results = {
        "Python EMT": [
            verify.Result("python_emt", 1, 1.0, 10.0),
            verify.Result("python_emt", 2, 1.0, 12.0),
        ],
        "Rust CLI": [verify.Result("rust_cli", 1, 1.0, 11.0)],
    }

    analysis = verify.print_results(all_results)

    assert analysis["python_vs_rust"]["within_tolerance"] is True
    output = capsys.readouterr().out
    assert "RESULTS" in output
    assert "Pairwise comparison" in output
    assert "Acceptance criterion" in output


def test_print_helpers_handle_empty_or_zero_reference_results(capsys):
    assert verify.print_method_statistics({"empty": []}) == {}

    verify.print_pairwise_comparison({"only": 1.0})
    verify.print_pairwise_comparison({"zero": 0.0, "other": 3.0})
    verify.print_iteration_table(
        {"left": [], "right": [verify.Result("right", 1, 1, 2)]}
    )
    verify.print_acceptance_summary({"python_vs_rust": None})

    output = capsys.readouterr().out
    assert "RESULTS" in output
    assert "Pairwise comparison" in output
    assert "right" in output


def test_write_results_serializes_metadata_analysis_and_dataclass_results(tmp_path):
    output_path = tmp_path / "nested" / "results.json"
    metadata = {"host": "test-host"}
    analysis = {"python_vs_rust": None}
    all_results = {
        "Python EMT": [
            verify.Result(
                method="python_emt",
                iteration=1,
                duration=1.2,
                total_energy_j=3.4,
                details={"workload_pid": 123},
            )
        ]
    }

    verify.write_results(output_path, metadata, analysis, all_results)

    payload = json.loads(output_path.read_text(encoding="utf-8"))
    assert payload["metadata"] == metadata
    assert payload["analysis"] == analysis
    assert payload["Python EMT"][0]["total_energy_j"] == pytest.approx(3.4)
    assert payload["Python EMT"][0]["details"] == {"workload_pid": 123}


def test_print_verification_header_includes_methods_and_rapl(monkeypatch, capsys):
    monkeypatch.setattr(
        verify,
        "print_metadata_summary",
        lambda metadata: print(f"metadata:{metadata['hostname']}"),
    )

    verify.print_verification_header(
        methods=[("Python EMT", lambda _duration, _iteration: None)],
        num_iterations=2,
        workload_duration=5.0,
        rapl_entries=[Path("/sys/class/powercap/intel-rapl:0")],
        metadata={"hostname": "host-a"},
        use_sudo=True,
    )

    output = capsys.readouterr().out
    assert "Methods: Python EMT" in output
    assert "RAPL zones: intel-rapl:0" in output
    assert "Rust CLI sudo: enabled" in output
    assert "metadata:host-a" in output


def test_run_verification_wires_collection_reporting_and_output(monkeypatch, tmp_path):
    calls = []
    output_path = tmp_path / "verification.json"
    fake_methods = [("Fake", lambda _duration, _iteration: None)]
    fake_results = {"Fake": [verify.Result("fake", 1, 0.1, 1.0)]}
    fake_analysis = {"python_vs_rust": None}

    monkeypatch.setattr(
        verify,
        "assert_rapl_available",
        lambda: calls.append("rapl") or [Path("/rapl/intel-rapl:0")],
    )
    monkeypatch.setattr(
        verify,
        "collect_run_metadata",
        lambda rapl_entries, iterations, duration, output, use_sudo: calls.append(
            ("metadata", rapl_entries, iterations, duration, output, use_sudo)
        )
        or {"hostname": "host-a"},
    )
    monkeypatch.setattr(
        verify,
        "build_verification_methods",
        lambda use_sudo: calls.append(("methods", use_sudo)) or fake_methods,
    )
    monkeypatch.setattr(
        verify,
        "print_verification_header",
        lambda *args: calls.append(("header", args)),
    )
    monkeypatch.setattr(
        verify,
        "run_methods",
        lambda methods, iterations, duration: calls.append(
            ("run", methods, iterations, duration)
        )
        or fake_results,
    )
    monkeypatch.setattr(
        verify,
        "print_results",
        lambda results: calls.append(("print", results)) or fake_analysis,
    )
    monkeypatch.setattr(
        verify,
        "write_results",
        lambda output, metadata, analysis, results: calls.append(
            ("write", output, metadata, analysis, results)
        ),
    )

    verify.run_verification(
        num_iterations=3,
        workload_duration=4.0,
        output_path=output_path,
        use_sudo=True,
    )

    assert calls[0] == "rapl"
    assert ("methods", True) in calls
    assert ("run", fake_methods, 3, 4.0) in calls
    assert ("print", fake_results) in calls
    assert calls[-1] == (
        "write",
        output_path,
        {"hostname": "host-a"},
        fake_analysis,
        fake_results,
    )


def test_measure_method_catches_subprocess_errors(monkeypatch, capsys):
    monkeypatch.setattr(verify.time, "sleep", lambda _seconds: None)

    results = verify.measure_method(
        "Subprocess",
        lambda _duration, _iteration: (_ for _ in ()).throw(
            subprocess.SubprocessError("subprocess failed")
        ),
        num_iterations=1,
        workload_duration=1.0,
    )

    assert results == []
    assert "subprocess failed" in capsys.readouterr().out


def test_prometheus_probe_parses_system_and_workload_cpu_metrics(monkeypatch):
    metrics_text = """
# HELP emt_energy_joules_total Energy
emt_energy_joules_total{device="cpu",scope="system",socket="0"} 12.5
emt_power_watts{device="cpu",scope="system",socket="0"} 4.25
emt_energy_joules_total{device="cpu",scope="workload",socket="0",workload="pid:123",workload_name="python"} 7.0
emt_power_watts{device="cpu",scope="workload",socket="0",workload="pid:123",workload_name="python"} 3.5
emt_energy_joules_total{device="gpu",scope="system",socket="0"} 99
"""

    class FakeResponse:
        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, traceback):
            return False

        def read(self):
            return metrics_text.encode("utf-8")

    monkeypatch.setattr(
        cadence_probe.urllib.request,
        "urlopen",
        lambda _url, timeout: FakeResponse(),
    )

    system_cpu, workloads = cadence_probe.read_metrics("http://example.test/metrics")

    assert system_cpu.energy_joules == pytest.approx(12.5)
    assert system_cpu.power_watts == pytest.approx(4.25)
    assert workloads["pid:123"] == cadence_probe.PowerReading(7.0, 3.5, "python")


def test_prometheus_probe_prefers_expected_workload_pid_over_busy_host_workload():
    samples = [
        cadence_probe.MetricsSample(
            0,
            cadence_probe.PowerReading(10.0, 0.0),
            {
                "pid:123": cadence_probe.PowerReading(1.0, 0.0),
                "pgrp:busy": cadence_probe.PowerReading(10.0, 0.0),
            },
        ),
        cadence_probe.MetricsSample(
            1,
            cadence_probe.PowerReading(12.0, 2.0),
            {
                "pid:123": cadence_probe.PowerReading(2.0, 1.0),
                "pgrp:busy": cadence_probe.PowerReading(50.0, 40.0),
            },
        ),
    ]

    assert cadence_probe.select_workload(samples, "pid:123") == "pid:123"
    assert cadence_probe.select_workload(samples, None) == "pgrp:busy"


def test_prometheus_probe_starts_exporter_for_workload_pid(monkeypatch):
    calls = []
    args = SimpleNamespace(
        emt_bin=Path("/tmp/emt"),
        host="127.0.0.1",
        port=19104,
        rate=4.0,
        scan_interval=1.0,
    )

    class FakeProcess:
        pass

    def fake_popen(command, **kwargs):
        calls.append((command, kwargs))
        return FakeProcess()

    monkeypatch.setattr(cadence_probe.subprocess, "Popen", fake_popen)

    cadence_probe.start_exporter(args, workload_pid=123)

    command, kwargs = calls[0]
    assert command[command.index("--pid") + 1] == "123"
    assert kwargs["cwd"] == cadence_probe.PROJECT_ROOT
    assert kwargs["start_new_session"] is True


def test_prometheus_probe_evaluate_fails_on_power_reset_after_nonzero():
    samples = [
        cadence_probe.MetricsSample(
            0,
            cadence_probe.PowerReading(10.0, 0.0),
            {"pid:123": cadence_probe.PowerReading(4.0, 0.0)},
        ),
        cadence_probe.MetricsSample(
            1,
            cadence_probe.PowerReading(12.0, 2.0),
            {"pid:123": cadence_probe.PowerReading(5.0, 1.0)},
        ),
        cadence_probe.MetricsSample(
            2,
            cadence_probe.PowerReading(12.0, 0.0),
            {"pid:123": cadence_probe.PowerReading(5.0, 0.0)},
        ),
    ]

    ok, summaries = cadence_probe.evaluate(
        samples,
        "pid:123",
        min_energy_changes=1,
        min_post_power_samples=1,
    )

    assert ok is False
    assert "power reset to zero" in summaries[0]
