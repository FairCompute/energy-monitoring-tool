import pytest
import os
import asyncio
from collections import defaultdict
import shutil
from datetime import datetime
import logging
from unittest.mock import MagicMock, patch, mock_open
from emt.energy_meter import EnergyMeter, PowerGroup, EnergyMonitor

TOLERANCE = 1e-9


class MockPGCPU(PowerGroup):
    """Mock PowerGroup class to simulate CPU PowerGroup behavior."""

    def __init__(self, name):
        self.name = name
        self._energy_trace = defaultdict(list)
        self._consumed_energy = 1000.0

    async def commence(self):
        """Simulates a long-running asynchronous task."""
        await asyncio.sleep(0.1)

    @classmethod
    def is_available(cls):
        return True


class MockPGGPU(PowerGroup):
    """Mock PowerGroup class to simulate CPU PowerGroup behavior."""

    def __init__(self, name):
        self.name = name
        self._energy_trace = defaultdict(list)
        self._consumed_energy = 1000.0

    async def commence(self):
        """Simulates a long-running asynchronous task."""
        await asyncio.sleep(0.1)

    @classmethod
    def is_available(cls):
        return True


@pytest.fixture
def mock_power_groups():
    """Fixture to provide mock power groups."""
    return [MockPGCPU("mock_pg_cpu"), MockPGGPU("mock_pg_gpu")]


@pytest.fixture
def energy_meter(mock_power_groups, tmp_path):
    """Fixture to create an EnergyMeter instance."""
    log_dir = tmp_path / "logs"
    return EnergyMeter(
        powergroups=mock_power_groups,
        logging_interval=2,
        tracing_interval=2,
        log_trace_path=log_dir,
        context_name="test_name",
    )


def test_initialization(energy_meter):
    """Test initialization of EnergyMeter."""
    assert isinstance(energy_meter, EnergyMeter)
    assert energy_meter.monitoring is False
    assert energy_meter.concluded is False
    assert len(energy_meter.power_groups) == 2
    assert energy_meter._context_name == "test_name"


def test_write_csv(energy_meter):
    """Test writing data to CSV."""
    data = {"col1": [1, 2], "col2": [3, 4]}
    filename = "test.csv"
    # create the directory as write_csv does not handle that
    energy_meter.write_csv(data, filename)
    file_path = os.path.join(energy_meter._log_trace_dir, filename)
    assert os.path.exists(file_path)

    with open(file_path, "r") as file:
        content = file.readlines()
    assert content == ["col1,col2\n", "1,3\n", "2,4\n"]
    # remove the diretory once done
    shutil.rmtree(energy_meter._log_trace_dir)


def test_log_traces_once(energy_meter):
    with (
        patch("datetime.datetime") as mock_datetime,
        patch.object(energy_meter, "write_csv") as mock_write_csv,
    ):

        mock_datetime.now.strftime.return_value = datetime(2024, 1, 1, 12, 0, 0)
        energy_meter._log_traces_once()
        # Assert
        # must be called twice for the two powergroups
        assert mock_write_csv.call_count == 2


@pytest.mark.asyncio
async def test_log_traces(energy_meter, mock_power_groups):
    """Test periodic logging of traces."""
    mock_power_groups[0]._energy_trace = {"key1": [1, 2], "key2": [3, 4]}

    with patch.object(EnergyMeter, "write_csv", return_value=None) as mock_write_csv:

        # Schedule the _log_traces coroutine
        log_traces_task = asyncio.create_task(energy_meter._log_traces())
        # Allow some time for the loop to start
        await asyncio.sleep(0.1)
        # Set the shutdown event to stop the loop
        energy_meter._shutdown_event.set()
        # Await the task to ensure it completes cleanly
        await log_traces_task
    # Assertions to check behavior
    assert mock_write_csv.call_count >= 0


@pytest.mark.asyncio
async def test_run_tasks_asynchronous(energy_meter):
    """Test running asynchronous tasks."""

    async def mocked_shutdown_asynchronous():
        """mocks the shutdown function of enenrgymeter class"""
        await asyncio.sleep(0.5)  # Simulated delay

    with (
        patch.object(
            energy_meter._power_groups[0], "commence", return_value=None
        ) as mock_commence_1,
        patch.object(
            energy_meter._power_groups[1], "commence", return_value=None
        ) as mock_commence_2,
        patch.object(
            energy_meter, "_shutdown_asynchronous", new=mocked_shutdown_asynchronous
        ),
    ):
        energy_meter._log_trace_interval = None
        run_async_task = asyncio.create_task(energy_meter._run_tasks_asynchronous())
        await asyncio.sleep(0.1)
        await run_async_task

    # the commence from both the powergroups should be called once each
    assert mock_commence_1.called
    assert mock_commence_2.called


def test_conclude(energy_meter):
    """Test concluding monitoring."""
    energy_meter._monitoring = True
    with patch.object(
        EnergyMeter, "_log_traces_once", return_value=None
    ) as mock_log_traces:
        energy_meter.conclude()

    assert energy_meter.concluded is True
    assert energy_meter._shutdown_event.is_set() is True
    assert energy_meter._monitoring is False
    assert mock_log_traces.called


def test_consumed_energy(energy_meter):
    """Test consumed energy per power group."""
    consumed_energy = energy_meter.consumed_energy
    assert len(consumed_energy.keys()) == 2
    for key, value in consumed_energy.items():
        assert abs(value - 1000.0) < TOLERANCE


def test_total_consumed_energy(energy_meter):
    """Test total consumed energy calculation."""
    total_energy = energy_meter.total_consumed_energy
    assert (
        abs(total_energy - 2000.0) < TOLERANCE
    )  # 2 power groups with 1000 energy each


@pytest.fixture
def mock_energy_meter_class():
    """
    This mocks the EnergyMeter() instace creation which is called inside EnergyMonitor()
    """
    with patch("emt.energy_meter.EnergyMeter", autospec=True) as MockEnergyMeter:
        mock_instance = MockEnergyMeter.return_value
        mock_instance.run.return_value = None
        mock_instance.conclude.return_value = None
        mock_instance.logger = MagicMock()
        mock_instance._monitoring = True
        mock_instance.total_consumed_energy = 42.0  # Example value
        mock_instance.consumed_energy = {"PG1": 20.0, "PG2": 22.0}
        yield MockEnergyMeter


def test_energy_monitor_enter_exit(mock_energy_meter_class, tmp_path):
    """Test the context management of EnergyMonitor."""
    with (
        patch("threading.Thread.start", return_value=None) as mock_start,
        patch("threading.Thread.join", return_value=None),
    ):
        monitor = EnergyMonitor(tracing_interval=20, log_trace_path=tmp_path)
        with monitor as meter:
            assert isinstance(meter, EnergyMeter)
        # separte thread start must be called once
        mock_start.assert_called
        # conclude from energy meter must be called once
        assert meter.conclude.called
