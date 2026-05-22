import pytest
from unittest.mock import patch, mock_open, MagicMock
from pathlib import Path
from emt.power_groups import RAPLSoC
from emt.power_groups.rapl import DeltaReader, extract_components

TOLERANCE = 1e-9

RAPL_DIR = "/sys/class/powercap"


# ---------------------------------------------------------------------------
# Tests for extract_components
# ---------------------------------------------------------------------------


def _p(name: str) -> Path:
    """Helper: build a Path inside RAPL_DIR from a zone name."""
    return Path(RAPL_DIR, name)


class TestExtractComponents:
    """Unit tests for the extract_components helper function."""

    # ------------------------------------------------------------------
    # Basic threshold tests
    # ------------------------------------------------------------------

    def test_unrelated_zone_name_returns_empty(self):
        """A zone path whose name does not prefix any entry in all_zones returns empty.

        Note: 'other-domain' does not start with 'intel-rapl:', so none of the
        intel-rapl entries will match and the result is empty.
        """
        zone = _p("other-domain")
        all_zones = [_p("intel-rapl:0"), _p("intel-rapl:0:0")]
        result = extract_components(zone, all_zones)
        assert result == []

    def test_one_colon_zone_name_is_top_level_not_a_subcomponent(self):
        """A zone with exactly one colon (e.g., intel-rapl:0) is a top-level package,
        not a sub-component, and should not appear in its own component list."""
        zone = _p("intel-rapl:0")
        all_zones = [_p("intel-rapl:0"), _p("intel-rapl:1")]
        result = extract_components(zone, all_zones)
        assert result == []

    def test_more_than_one_colon_is_a_subcomponent(self):
        """A zone with more than one colon (e.g., intel-rapl:0:0) is a sub-component."""
        zone = _p("intel-rapl:0")
        all_zones = [
            _p("intel-rapl:0"),
            _p("intel-rapl:0:0"),
        ]
        result = extract_components(zone, all_zones)
        assert result == [_p("intel-rapl:0:0")]

    # ------------------------------------------------------------------
    # Regression: flat sysfs layout and rglob traversal of RAPL zones
    # ------------------------------------------------------------------

    def test_flat_layout_sibling_subcomponents_are_discovered(self):
        """Regression: a previous implementation relied on rglob() directory traversal
        and expected sub-components to be nested under their parent zone directory.
        On systems where zones and their sub-components are siblings in a flat sysfs
        layout, that traversal only saw top-level zones and missed valid sub-components.
        The new extract_components function works correctly for the flat layout."""
        zone = _p("intel-rapl:0")
        # intel-rapl:0:0 has exactly 2 colons; it is a valid sub-component
        all_zones = [_p("intel-rapl:0"), _p("intel-rapl:0:0"), _p("intel-rapl:0:1")]
        result = extract_components(zone, all_zones)
        assert _p("intel-rapl:0:0") in result
        assert _p("intel-rapl:0:1") in result

    # ------------------------------------------------------------------
    # Multiple zones and correct assignment
    # ------------------------------------------------------------------

    def test_subcomponents_correctly_assigned_to_parent_zone(self):
        """Sub-components are matched to the correct parent zone only."""
        zone0 = _p("intel-rapl:0")
        zone1 = _p("intel-rapl:1")
        all_zones = [
            _p("intel-rapl:0"),
            _p("intel-rapl:0:0"),  # cores for socket 0
            _p("intel-rapl:0:1"),  # uncore for socket 0
            _p("intel-rapl:1"),
            _p("intel-rapl:1:0"),  # cores for socket 1
        ]
        comps0 = extract_components(zone0, all_zones)
        comps1 = extract_components(zone1, all_zones)

        assert _p("intel-rapl:0:0") in comps0
        assert _p("intel-rapl:0:1") in comps0
        assert _p("intel-rapl:1:0") not in comps0

        assert _p("intel-rapl:1:0") in comps1
        assert _p("intel-rapl:0:0") not in comps1
        assert _p("intel-rapl:0:1") not in comps1

    def test_no_subcomponents_returns_empty_list(self):
        """When all_zones contains only top-level zones, components should be empty."""
        zone = _p("intel-rapl:0")
        all_zones = [_p("intel-rapl:0"), _p("intel-rapl:1")]
        result = extract_components(zone, all_zones)
        assert result == []

    # ------------------------------------------------------------------
    # Edge cases / malformed inputs
    # ------------------------------------------------------------------

    def test_empty_all_zones_returns_empty(self):
        """Empty input list always produces empty output."""
        zone = _p("intel-rapl:0")
        assert extract_components(zone, []) == []

    def test_subcomponent_does_not_match_similar_but_different_zone(self):
        """intel-rapl:0:0 must NOT appear as sub-component of intel-rapl:00."""
        zone = _p("intel-rapl:00")
        all_zones = [_p("intel-rapl:0:0"), _p("intel-rapl:00")]
        result = extract_components(zone, all_zones)
        # intel-rapl:0:0 starts with "intel-rapl:0:" not "intel-rapl:00:"
        assert result == []

    def test_all_prefix_matching_descendants_are_returned(self):
        """Sub-sub-components (three colons) are still returned because they start
        with the parent prefix; they are valid deep sub-domains."""
        zone = _p("intel-rapl:0")
        all_zones = [
            _p("intel-rapl:0"),
            _p("intel-rapl:0:0"),
            _p("intel-rapl:0:0:0"),  # hypothetical deep sub-domain
        ]
        result = extract_components(zone, all_zones)
        # both are prefixed by "intel-rapl:0:"
        assert _p("intel-rapl:0:0") in result
        assert _p("intel-rapl:0:0:0") in result


# ---------------------------------------------------------------------------
# Tests for RAPLSoC that exercise component wiring via extract_components
# ---------------------------------------------------------------------------


def test_rapl_soc_with_subcomponents_wires_readers_correctly():
    """RAPLSoC should discover sub-components from the flat powercap listing and
    populate _components correctly without relying on rglob."""
    listdir_entries = [
        "intel-rapl:0",
        "intel-rapl:0:0",  # sub-domain for socket 0
        "intel-rapl:0:1",  # sub-domain for socket 0
        "intel-rapl:1",
    ]

    def fake_open(path, *args, **kwargs):
        name_map = {
            "/sys/class/powercap/intel-rapl:0/name": "package-0",
            "/sys/class/powercap/intel-rapl:1/name": "package-1",
        }
        path_str = str(path)
        read_data = name_map.get(path_str, "unknown")
        return mock_open(read_data=read_data)()

    with (
        patch("os.listdir", return_value=listdir_entries),
        patch("builtins.open", side_effect=fake_open),
        patch(
            "emt.utils.config.load_config",
            return_value={"measurement_units": {"energy": "Joules", "power": "Watts"}},
        ),
        patch("psutil.Process"),
    ):
        soc = RAPLSoC()

    assert soc.zones_count == 2
    # Two zone_readers for the two package zones
    assert len(soc.zone_readers) == 2
    # Sub-components for socket 0: intel-rapl:0:0 and intel-rapl:0:1
    assert len(soc._components[0]) == 2
    # Socket 1 has no sub-components in this listing
    assert len(soc._components[1]) == 0


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
        patch(
            "emt.utils.config.load_config",
            return_value={"measurement_units": {"energy": "Joules", "power": "Watts"}},
        ),
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


def test_rapl_soc_is_available_requires_readable_zone_files():
    """RAPL exists only when top-level zone metadata and energy are readable."""
    with (
        patch("os.path.exists", return_value=True),
        patch("os.listdir", return_value=["intel-rapl:0"]),
        patch("builtins.open", side_effect=PermissionError),
    ):
        assert RAPLSoC.is_available() is False


def test_rapl_soc_is_available_accepts_readable_top_level_zone():
    """Readable top-level package zones make the RAPL power group available."""
    with (
        patch("os.path.exists", return_value=True),
        patch("os.listdir", return_value=["intel-rapl:0", "intel-rapl:0:0"]),
        patch("builtins.open", mock_open(read_data="package-0")),
    ):
        assert RAPLSoC.is_available() is True


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
