import pytest

from scripts import benchmark_burst_attribution as burst


def test_expected_processes_extracts_root_and_child_lifetimes():
    events = [
        {"event": "process_start", "role": "root", "pid": 100},
        {
            "event": "child_spawned",
            "role": "child_over_interval",
            "pid": 101,
            "expected_runtime_seconds": 6.0,
            "sweep_start_offset_seconds": 10.0,
        },
        {
            "event": "process_end",
            "role": "child_over_interval",
            "pid": 101,
            "runtime_seconds": 6.0,
            "self_cpu_ticks_delta": 12,
        },
        {
            "event": "process_end",
            "role": "root",
            "pid": 100,
            "runtime_seconds": 20.0,
            "self_cpu_ticks_delta": 1,
        },
    ]

    expected = burst.expected_processes(events)

    assert expected == [
        burst.ExpectedProcess(
            pid=100,
            role="root",
            expected_runtime_seconds=20.0,
            sweep_repetition=None,
            sweep_start_offset_seconds=None,
            self_reported_cpu_ticks_delta=1,
        ),
        burst.ExpectedProcess(
            pid=101,
            role="child_over_interval",
            expected_runtime_seconds=6.0,
            sweep_repetition=None,
            sweep_start_offset_seconds=10.0,
            self_reported_cpu_ticks_delta=12,
        ),
    ]


def test_build_findings_marks_discovered_and_attributed_process_with_cpu_evidence():
    snapshot = {
        "workloads": [
            {
                "root_pid": 100,
                "group_id": "lineage:100",
                "energy": {"cpu_joules": 1.0, "dram_joules": 0.0, "gpu_joules": 0.0},
                "processes": [
                    {
                        "pid": 101,
                        "energy": {
                            "cpu_joules": 0.5,
                            "dram_joules": 0.25,
                            "gpu_joules": 0.0,
                        },
                    }
                ],
            }
        ]
    }
    expected = [
        burst.ExpectedProcess(
            pid=101,
            role="child_over_interval",
            expected_runtime_seconds=6.0,
            sweep_repetition=None,
            sweep_start_offset_seconds=None,
            self_reported_cpu_ticks_delta=20,
        )
    ]
    cpu_evidence = {
        101: burst.ProcCpuEvidence(
            pid=101,
            sample_count=3,
            first_cpu_ticks=10,
            last_cpu_ticks=25,
            observed_cpu_ticks=15,
            externally_observed=True,
        )
    }

    findings = burst.build_findings(snapshot, expected, cpu_evidence)

    assert findings == [
        burst.ProcessFinding(
            pid=101,
            role="child_over_interval",
            expected_runtime_seconds=6.0,
            sweep_repetition=None,
            sweep_start_offset_seconds=None,
            discovered=True,
            attributed=True,
            energy_joules=0.75,
            group_id="lineage:100",
            group_energy_joules=1.0,
            proc_cpu_ticks_observed=15,
            proc_cpu_sample_count=3,
            self_reported_cpu_ticks_delta=20,
            failure_mode="attributed",
        )
    ]


def test_build_findings_classifies_cpu_active_process_missed_by_snapshot():
    expected = [
        burst.ExpectedProcess(
            pid=101,
            role="child_under_interval",
            expected_runtime_seconds=1.0,
            sweep_repetition=None,
            sweep_start_offset_seconds=None,
            self_reported_cpu_ticks_delta=10,
        )
    ]

    findings = burst.build_findings({}, expected, {})

    assert findings[0].failure_mode == "not_discovered_before_exit_or_grouped_elsewhere"
    assert findings[0].attributed is False


def test_compact_group_evidence_keeps_expected_and_top_energy_groups():
    snapshot = {
        "workloads": [
            {
                "root_pid": 100,
                "group_id": "lineage:100",
                "energy": {"cpu_joules": 1.0, "dram_joules": 0.0, "gpu_joules": 0.0},
                "processes": [{"pid": 101, "energy": {"cpu_joules": 0.5}}],
            },
            {
                "root_pid": 200,
                "group_id": "lineage:200",
                "energy": {"cpu_joules": 10.0, "dram_joules": 0.0, "gpu_joules": 0.0},
                "processes": [],
            },
        ]
    }

    evidence = burst.compact_group_evidence(snapshot, {101}, top_limit=1)

    assert [group.group_id for group in evidence] == ["lineage:200", "lineage:100"]
    assert evidence[0].evidence_reason == "top_energy_group"
    assert evidence[1].evidence_reason == "expected_pid_intersection"


def test_lifetime_sweep_summary_requires_all_repetitions_reliable():
    reliable = burst.ProcessFinding(
        pid=101,
        role="sweep_child_52.0s",
        expected_runtime_seconds=52.0,
        sweep_repetition=1,
        sweep_start_offset_seconds=10.0,
        discovered=True,
        attributed=True,
        energy_joules=1.0,
        group_id="lineage:100",
        group_energy_joules=1.0,
        proc_cpu_ticks_observed=5,
        proc_cpu_sample_count=2,
        self_reported_cpu_ticks_delta=7,
        failure_mode="attributed",
    )
    missed = burst.ProcessFinding(
        pid=102,
        role="sweep_child_20.0s",
        expected_runtime_seconds=20.0,
        sweep_repetition=1,
        sweep_start_offset_seconds=10.0,
        discovered=True,
        attributed=False,
        energy_joules=0.0,
        group_id="lineage:100",
        group_energy_joules=1.0,
        proc_cpu_ticks_observed=5,
        proc_cpu_sample_count=2,
        self_reported_cpu_ticks_delta=7,
        failure_mode="discovered_without_energy_exit_accounting_or_sample_gap",
    )
    result = burst.PatternResult(
        pattern="lifetime_sweep",
        monitor_duration_seconds=90.0,
        workload_duration_seconds=60.0,
        collection_rate_hz=0.1,
        scan_interval_secs=30.0,
        attribution_refresh_secs=10.0,
        expected_processes=[],
        findings=[reliable, missed],
        proc_cpu_evidence=[],
        group_evidence=[],
        snapshot_group_count=0,
        snapshot_group_energy_joules=0.0,
        workload_events=[],
        unattributed_joules=0.0,
        system_total_joules=1.0,
        recommendation="",
    )

    summary = burst.summarize([result])

    assert summary["minimum_reliable_lifetime_seconds"] == pytest.approx(52.0)
    assert summary["lifetime_sweep"][0]["runtime_seconds"] == pytest.approx(20.0)
    assert summary["lifetime_sweep"][0]["start_offsets_seconds"] == [
        pytest.approx(10.0)
    ]
    assert summary["lifetime_sweep"][0]["reliable"] is False
    assert summary["patterns"]["lifetime_sweep"]["non_root_expected"] == 2
    assert summary["patterns"]["lifetime_sweep"]["non_root_attributed"] == 1


def test_recommendation_distinguishes_sub_interval_misses():
    findings = [
        burst.ProcessFinding(
            pid=101,
            role="child_under_interval",
            expected_runtime_seconds=0.5,
            sweep_repetition=None,
            sweep_start_offset_seconds=None,
            discovered=False,
            attributed=False,
            energy_joules=0.0,
            group_id=None,
            group_energy_joules=0.0,
            proc_cpu_ticks_observed=3,
            proc_cpu_sample_count=2,
            self_reported_cpu_ticks_delta=4,
            failure_mode="not_discovered_before_exit_or_grouped_elsewhere",
        )
    ]

    recommendation = burst.recommendation_for("child_under_interval", findings, 1.0)

    assert "Sub-interval processes" in recommendation
    assert "re-benchmarked" in recommendation
