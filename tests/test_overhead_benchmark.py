from scripts import benchmark_emt_overhead as overhead


def test_rapl_delta_handles_counter_wrap():
    before = {
        "zone": {
            "name": "package-0",
            "energy_uj": 900,
            "max_energy_uj": 1000,
        }
    }
    after = {
        "zone": {
            "name": "package-0",
            "energy_uj": 100,
            "max_energy_uj": 1000,
        }
    }

    total, zones = overhead.rapl_delta_joules(before, after)

    assert total == 0.0002
    assert zones == [{"path": "zone", "name": "package-0", "delta_joules": 0.0002}]


def test_snapshot_diagnostics_finds_emt_group_and_workload_groups():
    snapshot = {
        "tracked_pids": [10, 11, 20],
        "workloads": [
            {
                "root_pid": 10,
                "group_id": "lineage:10",
                "percentage_of_system": 0.4,
                "energy": {"cpu_joules": 1.0, "dram_joules": 0.5, "gpu_joules": 0.0},
                "processes": [{"pid": 11, "energy": {"cpu_joules": 0.2}}],
            },
            {
                "root_pid": 20,
                "group_id": "lineage:20",
                "percentage_of_system": 99.6,
                "energy": {"cpu_joules": 20.0, "dram_joules": 0.0, "gpu_joules": 0.0},
                "processes": [],
            },
        ],
        "diagnostics": {
            "collection_ticks": 7,
            "process_scans": 3,
            "process_groups": 2,
        },
    }

    diagnostics = overhead.snapshot_diagnostics(
        snapshot,
        {10, 11},
        [20],
    )

    assert diagnostics["emt_group_found"] is True
    assert diagnostics["emt_group_ids"] == ["lineage:10"]
    assert diagnostics["emt_percentage"] == 0.4
    assert diagnostics["emt_energy_joules"] == 1.5
    assert diagnostics["workload_groups_found"] == 1
    assert diagnostics["workload_group_ids"] == ["lineage:20"]
    assert diagnostics["groups_distinct"] is True
    assert diagnostics["tracked_pid_count"] == 3
    assert diagnostics["collection_tick_count"] == 7
    assert diagnostics["process_scan_count"] == 3
    assert diagnostics["process_group_count"] == 2


def test_snapshot_diagnostics_flags_merged_emt_and_workload_group():
    snapshot = {
        "tracked_pids": [10, 20],
        "workloads": [
            {
                "root_pid": 10,
                "group_id": "cgroup:merged",
                "percentage_of_system": 100.0,
                "energy": {"cpu_joules": 10.0, "dram_joules": 0.0, "gpu_joules": 0.0},
                "processes": [{"pid": 20, "energy": {"cpu_joules": 5.0}}],
            }
        ],
    }

    diagnostics = overhead.snapshot_diagnostics(snapshot, {10}, [20])

    assert diagnostics["groups_distinct"] is False


def run_result(
    workload: str, iteration: int, overhead_percent: float
) -> overhead.RunResult:
    return overhead.RunResult(
        mode="tui",
        workload=workload,
        iteration=iteration,
        duration_seconds=60.0,
        emt_pid=10,
        workload_pids=[20],
        raw_package_dram_joules=100.0,
        emt_cpu_seconds=0.1,
        system_active_cpu_seconds=100.0,
        emt_cpu_share=0.001,
        external_estimated_emt_joules=0.1,
        external_overhead_percent=overhead_percent,
        peak_rss_bytes=1024,
        rss_samples_bytes=[1000, 1024],
        process_sample_count=2,
        min_process_count=1,
        max_process_count=2,
        pids_seen=[10],
        tracked_pid_count=5,
        collection_tick_count=7,
        process_scan_count=3,
        snapshot_process_group_count=2,
        snapshot_emt_group_found=True,
        snapshot_emt_group_ids=["lineage:10"],
        snapshot_emt_percentage=overhead_percent,
        snapshot_emt_energy_joules=0.1,
        snapshot_workload_groups_found=1,
        snapshot_workload_group_ids=["lineage:20"],
        snapshot_groups_distinct=True,
        displayed_vs_external_delta_percent=0.0,
        displayed_external_agree=True,
        displayed_attribution_diagnostic="display_agrees_with_external_estimate",
        collection_rate_hz=0.1,
        scan_interval_secs=30.0,
        render_interval_millis=2000,
        rapl_zones=[],
    )


def test_overhead_summary_requires_three_non_idle_runs():
    summary = overhead.summarize(
        [
            run_result("single_cpu", 1, 0.1),
            run_result("multi_cpu", 1, 0.2),
        ]
    )

    assert summary["acceptance"]["tui_non_idle_overhead_passed"] is False


def test_overhead_summary_enforces_tui_non_idle_thresholds():
    results = [
        run_result("single_cpu", 1, 0.1),
        run_result("single_cpu", 2, 0.2),
        run_result("single_cpu", 3, 0.3),
        run_result("multi_cpu", 1, 0.1),
        run_result("multi_cpu", 2, 0.2),
        run_result("multi_cpu", 3, 0.3),
    ]

    summary = overhead.summarize(results)

    assert summary["acceptance"]["tui_non_idle_overhead_passed"] is True
    assert summary["tui"]["single_cpu"]["grouping_distinct"] is True
    assert summary["tui"]["single_cpu"]["max_visible_emt_percent"] == 0.3
    assert (
        summary["tui"]["single_cpu"]["median_displayed_vs_external_delta_percent"]
        == 0.0
    )


def test_overhead_summary_rejects_visible_tui_percentage_over_one_percent():
    results = [
        run_result("single_cpu", 1, 0.1),
        run_result("single_cpu", 2, 0.2),
        run_result("single_cpu", 3, 0.3),
        run_result("multi_cpu", 1, 0.1),
        run_result("multi_cpu", 2, 0.2),
        run_result("multi_cpu", 3, 0.3),
    ]
    results[0].snapshot_emt_percentage = 1.01

    summary = overhead.summarize(results)

    assert summary["acceptance"]["tui_non_idle_overhead_passed"] is False
    assert summary["tui"]["single_cpu"]["max_visible_emt_percent"] == 1.01
