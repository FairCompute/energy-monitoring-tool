import importlib.util
from pathlib import Path

VERIFY_PATH = Path(__file__).resolve().parents[1] / "verification" / "verify.py"
VERIFY_SPEC = importlib.util.spec_from_file_location("verification_verify", VERIFY_PATH)
VERIFY_MODULE = importlib.util.module_from_spec(VERIFY_SPEC)
assert VERIFY_SPEC.loader is not None
VERIFY_SPEC.loader.exec_module(VERIFY_MODULE)

Result = VERIFY_MODULE.Result
assert_rapl_available = VERIFY_MODULE.assert_rapl_available
build_acceptance_analysis = VERIFY_MODULE.build_acceptance_analysis
rapl_energy_entries = VERIFY_MODULE.rapl_energy_entries


def test_rapl_energy_entries_only_returns_counter_directories(tmp_path):
    rapl_package = tmp_path / "intel-rapl:0"
    rapl_package.mkdir()
    (rapl_package / "energy_uj").write_text("123")

    rapl_name_only = tmp_path / "intel-rapl:1"
    rapl_name_only.mkdir()
    (rapl_name_only / "name").write_text("package-1")

    unrelated = tmp_path / "not-rapl"
    unrelated.mkdir()
    (unrelated / "energy_uj").write_text("456")

    assert rapl_energy_entries(tmp_path) == [rapl_package]


def test_assert_rapl_available_raises_when_powercap_is_empty(tmp_path):
    msg = "No readable RAPL energy counters were found"
    try:
        assert_rapl_available(tmp_path)
    except RuntimeError as exc:
        assert msg in str(exc)
    else:
        raise AssertionError("Expected assert_rapl_available to raise RuntimeError")


def test_build_acceptance_analysis_marks_python_and_rust_as_within_tolerance():
    all_results = {
        "Python EMT": [Result("python_emt", 1, 30.0, 100.0)],
        "Rust CLI": [Result("rust_cli", 1, 30.0, 101.5)],
    }

    analysis = build_acceptance_analysis(all_results)
    python_vs_rust = analysis["python_vs_rust"]

    assert python_vs_rust["iterations_compared"] == 1
    assert python_vs_rust["within_tolerance"] is True
    assert python_vs_rust["relative_diff_percent"] == 1.5


def test_build_acceptance_analysis_uses_mean_across_multiple_iterations():
    all_results = {
        "Python EMT": [
            Result("python_emt", 1, 30.0, 100.0),
            Result("python_emt", 2, 30.0, 102.0),
        ],
        "Rust CLI": [
            Result("rust_cli", 1, 30.0, 101.0),
            Result("rust_cli", 2, 30.0, 103.0),
        ],
    }

    analysis = build_acceptance_analysis(all_results)
    python_vs_rust = analysis["python_vs_rust"]

    assert python_vs_rust["python_mean_j"] == 101.0
    assert python_vs_rust["rust_mean_j"] == 102.0
    assert round(python_vs_rust["relative_diff_percent"], 6) == round(100.0 / 101.0, 6)
    assert python_vs_rust["iterations_compared"] == 2


def test_build_acceptance_analysis_marks_python_and_rust_as_out_of_tolerance():
    all_results = {
        "Python EMT": [Result("python_emt", 1, 30.0, 100.0)],
        "Rust CLI": [Result("rust_cli", 1, 30.0, 103.0)],
    }

    analysis = build_acceptance_analysis(all_results)
    python_vs_rust = analysis["python_vs_rust"]

    assert python_vs_rust["within_tolerance"] is False
    assert python_vs_rust["relative_diff_percent"] == 3.0
