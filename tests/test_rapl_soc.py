import pytest
from unittest.mock import patch, mock_open, MagicMock
from pathlib import Path
from emt.power_groups import DeltaReader, RAPLSoC

TOLERANCE = 1e-9


# Test for DeltaReader
@pytest.fixture
def delta_reader():
    """Fixture to create a DeltaReader instance."""
    return DeltaReader("/fake/path", num_trails=3)


def test_delta_reader(delta_reader):
    """Test the DeltaReader for computing deltas."""
    # patch the built in open method with a mocked version
    with patch("builtins.open", mock_open(read_data="5000000")) as mock_file:
        delta_reader._previous_value = 0.0
        result = delta_reader()
        assert abs(result - 5.0) < TOLERANCE  # (5000000 - 4000000) * 1e-6 = 1.0 Joules
        # ensure that deltareader correctly attempts to read path with "r" mode
        mock_file.assert_called_once_with(Path("/fake/path/energy_uj"), "r")


def test_delta_reader_overflow(delta_reader):
    """Test DeltaReader handling counter overflow."""
    with patch("builtins.open", mock_open(read_data="3000000")) as mock_file:
        delta_reader._previous_value = 4000000
        result = delta_reader()
        assert abs(result - 0.0) < TOLERANCE
        mock_file.assert_called_with(Path("/fake/path/energy_uj"), "r")


def test_delta_reader_overflow_multiple_reads(delta_reader):
    """Test DeltaReader handling counter overflow correctly when the return value changes"""

    mock_file = mock_open()
    # ensure the two consecutive calls return two different values
    mock_file.return_value.read.side_effect = [
        "0",
        "8000000",
    ]  # First call returns 4000000, second 3000000
    with patch("builtins.open", mock_file) as mocked_file:
        delta_reader._previous_value = 4000000
        result = delta_reader()
        assert abs(result - 4.0) < TOLERANCE
        mocked_file.assert_called_with(Path("/fake/path/energy_uj"), "r")


# Test for RAPLSoC
@pytest.fixture
def rapl_soc():
    """Fixture to create a mocked RAPLSoC instance."""
    with (
        patch("os.listdir", return_value=["intel-rapl:0", "intel-rapl:1"]),
        patch("builtins.open", mock_open(read_data="fake_zone_name")),
    ):
        with patch("psutil.Process") as mocked_process:

            # Create a mock for the tracked parent process
            mock_tracked_process = MagicMock(pid=1234)
            mock_tracked_process.cpu_percent.return_value = (
                20  # ranges between 0 - 100* cpu_count
            )
            mock_tracked_process.memory_percent.return_value = 20

            # mock a child process
            mock_process_info_2 = MagicMock(pid=12341)
            mock_process_info_2.cpu_percent.return_value = (
                140  # ranges between 0 - 100* cpu_count
            )
            mock_process_info_2.memory_percent.return_value = 10

            mock_tracked_process.children.return_value = [mock_process_info_2]
            # Set the return value of the mocked Process class to the mock_tracked_process
            mocked_process.return_value = mock_tracked_process
            yield RAPLSoC()


def test_process_tracking(rapl_soc):
    # Retrieve the tracked PIDs
    tracked_ps = [rapl_soc.tracked_process] + rapl_soc.tracked_process.children(
        recursive=True
    )
    tracked_pids = [ps.pid for ps in tracked_ps]
    # Check that the tracked PIDs match the expected values
    expected_pids = [1234, 12341]
    assert (
        tracked_pids == expected_pids
    ), f"Expected PIDs {expected_pids}, but got {tracked_pids}"
    # Verify the tracked processes count
    assert len(tracked_pids) == 2  # Ensure there are 2 tracked processes


def test_rapl_soc_initialization(rapl_soc):
    """Test the RAPLSoC initialization."""
    assert rapl_soc.zones_count == 2
    assert len(rapl_soc._zones) == 2
    assert len(rapl_soc.zone_readers) == 2


def test_rapl_soc_read_energy(rapl_soc):
    """Test _read_energy in RAPLSoC."""
    with (
        patch.object(
            rapl_soc,
            "zone_readers",
            new=[MagicMock(return_value=1000.0), MagicMock(return_value=2000.0)],
        ),
        patch.object(
            rapl_soc,
            "dram_readers",
            new=[MagicMock(return_value=5000.0), MagicMock(return_value=5000.0)],
        ),
        patch.object(
            rapl_soc,
            "core_readers",
            new=[MagicMock(return_value=1000.0), MagicMock(return_value=3000.0)],
        ),
        patch.object(
            rapl_soc,
            "igpu_readers",
            new=[MagicMock(return_value=1000.0), MagicMock(return_value=1000.0)],
        ),
    ):

        energy = rapl_soc._read_energy()
        assert energy == {
            "zones": 3000.0,
            "cores": 4000.0,
            "dram": 10000.0,
            "igpu": 2000.0,
        }


def test_read_utilization(rapl_soc):
    """Test _read_utilization in RAPLSoC."""
    with (
        patch("psutil.cpu_percent", return_value=90),
        patch("psutil.cpu_count", return_value=2),
    ):  # , \
        #  patch.object(rapl_soc.tracked_process, "children", return_value=[]), \
        #  patch.object(rapl_soc.tracked_process, "cpu_percent", return_value=25):

        utilization = rapl_soc._read_utilization()
        assert utilization["cpu_util"] == 90
        assert abs(utilization["ps_util"] - 80) < TOLERANCE  # 160 / 2
        assert abs(utilization["norm_ps_util"] - (80 / 90)) < TOLERANCE  # 80 /90
        assert abs(utilization["dram"] - 30) < TOLERANCE  # 20 +10
