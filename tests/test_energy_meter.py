import pytest
import asyncio
from collections import defaultdict
import threading

from unittest.mock import MagicMock, patch, AsyncMock, ANY
from emt.energy_meter import EnergyMeter, PowerGroup, EnergyMonitor
from emt.utils import TraceRecorder

TOLERANCE = 1e-9


class MockPGCPU(PowerGroup):
    """Mock PowerGroup class to simulate CPU PowerGroup behavior."""

    def __init__(self):
        self.name = "RAPLSoC"
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

    def __init__(self):
        self.name = "NvidiaGPU"
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
    return [MockPGCPU(), MockPGGPU()]


@pytest.fixture
def energy_meter(mock_power_groups):
    """Fixture to create an EnergyMeter instance."""
    return EnergyMeter(
        powergroups=mock_power_groups,
        context_name="test_name",
        trace_recorders=None,
    )


def test_initialization(energy_meter):
    """Test initialization of EnergyMeter."""
    assert isinstance(energy_meter, EnergyMeter)
    assert energy_meter.monitoring is False
    assert energy_meter.concluded is False
    assert len(energy_meter.power_groups) == 2
    assert energy_meter._context_name == "test_name"
    assert energy_meter.trace_recorders == []


@pytest.mark.asyncio
async def test_run_tasks_asynchronous(energy_meter):
    """Test running asynchronous tasks."""

    async def mocked_shutdown_asynchronous():
        """mocks the shutdown function of EnergyMonitor class"""
        await asyncio.sleep(0.5)  # Simulated delay

    with (
        patch.object(
            energy_meter._power_groups[0], "commence", new_callable=AsyncMock
        ) as mock_commence_1,
        patch.object(
            energy_meter._power_groups[1], "commence", new_callable=AsyncMock
        ) as mock_commence_2,
        patch.object(
            energy_meter, "_shutdown_asynchronous", new=mocked_shutdown_asynchronous
        ),
    ):
        mock_trace_emitter_1 = AsyncMock()
        mock_trace_emitter_2 = AsyncMock()
        energy_meter.trace_recorders = [mock_trace_emitter_1, mock_trace_emitter_2]
        run_async_task = asyncio.create_task(energy_meter._run_tasks_asynchronous())
        await asyncio.sleep(0.1)
        await run_async_task

    # Assert `commence` is awaited for both power groups
    mock_commence_1.assert_awaited_once()
    mock_commence_2.assert_awaited_once()

    # Assert trace emitters are called
    mock_trace_emitter_1.assert_awaited_once()
    mock_trace_emitter_2.assert_awaited_once()


def test_run(energy_meter):
    """Test the synchronous `run` method of EnergyMeter."""

    # Mock the `_run_tasks_asynchronous` to avoid running actual asynchronous tasks
    with patch.object(
        energy_meter, "_run_tasks_asynchronous", new_callable=AsyncMock
    ) as mock_async_run:
        energy_meter_thread = threading.Thread(target=energy_meter.run)
        energy_meter_thread.start()

        energy_meter_thread.join()

    # Check that the `_run_tasks_asynchronous` was called once
    mock_async_run.assert_awaited_once()

    # Assertions for changes in the object state
    assert energy_meter._monitoring is True
    assert energy_meter._shutdown_event.is_set() is False


def test_conclude(energy_meter):
    """Test concluding monitoring."""
    energy_meter._monitoring = True
    mock_trace_emitter_1 = MagicMock()
    mock_trace_emitter_2 = MagicMock()

    energy_meter.trace_recorders = [mock_trace_emitter_1, mock_trace_emitter_2]
    energy_meter.conclude()
    assert energy_meter.concluded is True
    assert energy_meter._shutdown_event.is_set() is True
    assert energy_meter._monitoring is False
    assert len(energy_meter.trace_recorders) == 2

    # Assert that write_traces method is called on the mock trace emitter
    mock_trace_emitter_1.write_traces.assert_called_once()
    mock_trace_emitter_2.write_traces.assert_called_once()


def test_total_consumed_energy(energy_meter):
    """Test total consumed energy calculation."""
    total_energy = energy_meter.total_consumed_energy
    assert (
        abs(total_energy - 2000.0) < TOLERANCE
    )  # 2 power groups with 1000 energy each


def test_consumed_energy(energy_meter):
    """Test consumed energy per power group."""
    consumed_energy = energy_meter.consumed_energy
    assert len(consumed_energy.keys()) == 2
    for key, value in consumed_energy.items():
        assert abs(value - 1000.0) < TOLERANCE


def test_energy_monitor_initialization():
    """Test proper initialization of EnergyMonitor."""
    mock_trace_recorder = MagicMock(spec=TraceRecorder)
    mock_trace_recorder.__class__ = TraceRecorder

    # Valid initialization
    monitor = EnergyMonitor(name="test_context", trace_recorders=[mock_trace_recorder])
    assert monitor.context_name == "test_context"
    assert monitor._trace_recorders == [mock_trace_recorder]

    with (patch("logging.getLogger") as mock_logger,):
        mock_logger.return_value.hasHandlers.return_value = True
        monitor_no_traces = EnergyMonitor(
            name="context_no_traces", trace_recorders=None
        )
        assert monitor_no_traces._trace_recorders == []

    # Invalid trace recorder
    with pytest.raises(ValueError, match="Invalid trace emitters provided."):
        EnergyMonitor(
            trace_recorders="invalid_trace_recorder"
        )  # Non-TraceRecorder type


@pytest.fixture
def mock_power_group_class():
    """Fixture for mock PowerGroups to use in the test."""
    return [
        MockPGCPU().__class__,
        MockPGGPU().__class__,
    ]


@pytest.fixture
def mock_trace_recorders():
    """Fixture for mock TraceRecorders."""
    mock_trace_recorder_1 = MagicMock(spec=TraceRecorder)
    mock_trace_recorder_1.__class__ = TraceRecorder
    mock_trace_recorder_2 = MagicMock(spec=TraceRecorder)
    mock_trace_recorder_2.__class__ = TraceRecorder
    return [mock_trace_recorder_1, mock_trace_recorder_2]


def test_enter_method(mock_trace_recorders):
    """Test the __enter__ method of the EnergyMonitor."""
    # Create an instance of EnergyMonitor
    energy_monitor = EnergyMonitor(
        name="TestContext",
        trace_recorders=mock_trace_recorders,
    )
    with (
        patch("emt.energy_meter.logger", return_value=MagicMock()) as mock_logger,
        patch("threading.Thread", return_value=MagicMock()) as mock_thread,
        patch("time.sleep", return_value=None),
    ):
        energy_meter = energy_monitor.__enter__()

    # 1. Verify logging to ensure proper context
    mock_logger.info.assert_any_call(ANY)
    for trace_emitter in mock_trace_recorders:
        assert trace_emitter.trace_location is not None
    # 4. Thread starting should be mocked
    mock_thread.assert_called_once_with(
        name="EnergyMonitoringThread", target=energy_meter.run
    )
    assert energy_meter is not None


def test_exit_method(mock_trace_recorders):
    """Test the __enter__ method of the EnergyMonitor."""
    # Create an instance of EnergyMonitor
    monitor = EnergyMonitor(name="TestContext", trace_recorders=mock_trace_recorders)
    mock_energy_meter = MagicMock()
    mock_energy_meter.total_consumed_energy = 1000.0
    mock_energy_meter.consumed_energy = {"RAPLSoC": 500.0, "NvidiaGPU": 500.0}
    monitor.energy_meter = mock_energy_meter
    monitor.energy_meter_thread = MagicMock()
    monitor.start_time = 0
    # Invoke the __enter__ method (which gets triggered when used with the 'with' statement)
    with (
        patch("emt.energy_meter.logger", return_value=MagicMock()),
        patch("threading.Thread", return_value=MagicMock()),
        patch("logging.getLogger", return_value=MagicMock()),
    ):
        monitor.__exit__()

    # Conclude should be called on the energy meter
    monitor.energy_meter.conclude.assert_called_once()

    # Join the thread
    monitor.energy_meter_thread.join.assert_called_once()


if __name__ == "__main__":
    pytest.main([__file__])
