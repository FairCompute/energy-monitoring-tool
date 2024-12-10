import pytest
from unittest.mock import AsyncMock, patch, MagicMock
from collections import defaultdict
from emt.power_groups import NvidiaGPU, DeltaCalculator

# define global variabels
enregy_call_counter = 0
tolerance = 1e-9


class TestDeltaCalculator:
    def test_initialization(self):
        # Test default initialization
        calculator = DeltaCalculator()
        assert abs(calculator._init_energy - 0.0) < tolerance

        # Test initialization with a specific value
        calculator = DeltaCalculator(100.0)
        assert abs(calculator._init_energy - 100.0) < tolerance

    def test_call_method(self):
        calculator = DeltaCalculator()

        # First call should calculate difference from initial energy (0.0)
        result = calculator(5000.0)  # 5000 J input
        assert abs(result - 5.0) < tolerance  # (5000 - 0) / 1000

        # Second call should calculate difference from last input (5000.0)
        result = calculator(7000.0)  # 7000 J input
        assert abs(result - 2.0) < tolerance  # (7000 - 5000) / 1000

        # Third call with no previous input, should work from last input
        result = calculator(7000.0)  # 7000 J input
        assert abs(result - 0.0) < tolerance  # (7000 - 7000) / 1000

        # Ensure that the internal state updates correctly
        assert abs(calculator._init_energy - 7000.0) < tolerance

    def test_negative_energy(self):
        calculator = DeltaCalculator(5000.0)

        # Calculate energy with a lower current energy
        result = calculator(3000.0)  # 3000 J input
        assert abs(result + 2.0) < tolerance  # (3000 - 5000) / 1000

    def test_float_precision(self):
        calculator = DeltaCalculator(1.5)
        result = calculator(2.5)  # 2.5 J input
        assert abs(result - 0.001) < tolerance  # (2.5 - 1.5) / 1000


# Define the side effect for energy consumption based on device handles
def energy_call_side_effect(device_handle):
    """
    This function return the mock energy consumption for each handle.
    In every call, it increases the energy of the first zone by 1000 and the second zone by 2000.
    Also, increases the global energy_call_counter to keep track the increase in energy
    """
    # Counter to track calls
    global enregy_call_counter
    # Mock energy consumption value
    if device_handle == mock_device_handle_1:
        return_energy = 1000 * enregy_call_counter  # Energy for the first device handle
    elif device_handle == mock_device_handle_2:
        return_energy = (
            1000 * 2 * enregy_call_counter
        )  # Energy for the second device handle
    else:
        raise ValueError("Invalid device handle")
    enregy_call_counter += 1
    return return_energy


@pytest.fixture
def setup_gpu():
    # Mock the Process class from psutil
    with patch("psutil.Process") as mock_process_class:
        # Set up NVML mocks
        with patch("emt.power_groups.nvidia_gpu.pynvml") as mock_pynvml:
            # mock device counts # 2 gpus
            mock_pynvml.nvmlDeviceGetCount.return_value = 2
            # Mock the nvmlDeviceGetIndex to return unique indices for each GPU
            mock_pynvml.nvmlDeviceGetIndex.side_effect = [
                0,
                1,
            ]  # Unique indices for the two GPUs
            # Mock device handles
            global mock_device_handle_1
            global mock_device_handle_2

            mock_device_handle_1 = MagicMock()
            mock_device_handle_2 = MagicMock()
            # side_effect -> if called twice, it would return these two values one after the other
            mock_pynvml.nvmlDeviceGetHandleByIndex.side_effect = [
                mock_device_handle_1,
                mock_device_handle_2,
            ]

            # Mock the total energy consumption method
            mock_pynvml.nvmlDeviceGetTotalEnergyConsumption.side_effect = (
                energy_call_side_effect
            )

            # Mock memory info for each GPU
            mock_memory_info_1 = MagicMock(
                used=1024 * 1024 * 1024
            )  # 1 GB total memory for GPU 1
            mock_memory_info_2 = MagicMock(
                used=2048 * 1024 * 1024
            )  # 2 GB total memory for GPU 2
            mock_pynvml.nvmlDeviceGetMemoryInfo.side_effect = [
                mock_memory_info_1,
                mock_memory_info_2,
            ]

            # Mock running processes with different memory usage for each GPU
            mock_process_info_1 = MagicMock(
                pid=1234, usedGpuMemory=512 * 1024 * 1024
            )  # 512 MB used for GPU 1
            mock_process_info_2 = MagicMock(
                pid=1235, usedGpuMemory=256 * 1024 * 1024
            )  # 1 GB used for GPU 2
            mock_pynvml.nvmlDeviceGetComputeRunningProcesses.side_effect = [
                [mock_process_info_1],
                [mock_process_info_2],
            ]

            # Create a mock for the tracked_process
            mock_tracked_process = MagicMock()
            mock_tracked_process.children.return_value = [
                mock_process_info_1,
                mock_process_info_2,
            ]
            # Set the return value of the mocked Process class to the mock_tracked_process
            mock_process_class.return_value = mock_tracked_process

            # Initialize the NvidiaGPU instance
            gpu = NvidiaGPU()
            # Mock the total energy consumption method
            # This will make sure that DeltaCalculaor reads two different energy readings
            mock_pynvml.nvmlDeviceGetTotalEnergyConsumption.side_effect = (
                energy_call_side_effect
            )

            yield gpu, mock_pynvml


def test_process_tracking(setup_gpu):
    gpu, _ = setup_gpu
    # Retrieve the tracked PIDs
    tracked_pids = [proc.pid for proc in gpu.tracked_process.children()]
    # Check that the tracked PIDs match the expected values
    expected_pids = [1234, 1235]
    assert (
        tracked_pids == expected_pids
    ), f"Expected PIDs {expected_pids}, but got {tracked_pids}"
    # Verify the tracked processes count
    assert len(tracked_pids) == 2  # Ensure there are 2 tracked processes


def test_zones_are_individual(setup_gpu):
    gpu, _ = setup_gpu
    assert len(gpu.zones) == 2  # Ensure there are 2 zones
    assert len(gpu._zones) == 2  # Ensure internal _zones has 2 distinct handles
    # Further test that the zones are distinct
    assert gpu._zones[0] != gpu._zones[1]  # Check that the GPU handles are not the same
    assert gpu.zones[0] != gpu.zones[1]  # Check that the two zones are not the same


def test_initialization(setup_gpu):
    gpu, _ = setup_gpu
    assert len(gpu.zones) == 2  # Check that it found 2 GPUs
    assert len(gpu._zones) == 2
    assert len(gpu.processes) == 3  # (2 children process and 1 parent process)


def test_read_energy(setup_gpu):
    gpu, _ = setup_gpu
    energy_data = gpu._read_energy()
    # Check the expected output
    # since the function is called twice, because of two handles so delta = 2*(1,2)
    expected_energy_data = {
        0: 2.0,
        1: 4.0,
    }  # Adjust based on the expected delta energy calculation
    assert len(energy_data) == 2  # Ensure there are energy readings for both zones
    assert all(value in energy_data.values() for value in expected_energy_data.values())


def test_read_utilization(setup_gpu):
    gpu, _ = setup_gpu
    used_mem_zones, ps_mem_zones, ps_util_zones = gpu._read_utilization()
    exp_available_mem_zones = {0: 1024 * 1024 * 1024, 1: 2048 * 1024 * 1024}
    exp_ps_mem_zones = {0: 512 * 1024 * 1024, 1: 256 * 1024 * 1024}
    exp_ps_util_zones = {
        0: 0.5,
        1: 0.125,
    }  # Adjust based on the expected delta energy calculation
    assert len(ps_util_zones) == 2  # Ensure there are energy readings for both zones
    assert all(value in ps_util_zones.values() for value in exp_ps_util_zones.values())
    assert all(
        value in used_mem_zones.values() for value in exp_available_mem_zones.values()
    )
    assert all(value in ps_mem_zones.values() for value in exp_ps_mem_zones.values())


def test_utilized_energy_calcualtoin(setup_gpu):
    """
    Check the logic of energy utilization
    """
    gpu, _ = setup_gpu
    consumed_energy_zones = gpu._read_energy()
    _, _, ps_util_zones = gpu._read_utilization()
    if consumed_energy_zones.keys() != ps_util_zones.keys():
        raise ValueError("Dictionaries do not have the same zone_handle keys.")
    # get weighted sum of energy utilization
    consumed_utilized_energy = sum(
        consumed_energy_zones[zone] * ps_util_zones[zone]
        for zone in consumed_energy_zones
    )
    # energy - [2.0, 4.0] , utilization [0.5, 0.125] utilized energy = (2.0 * 0.5 + 4.0 * 0.125) = 1.5
    assert abs(consumed_utilized_energy - 1.5) < tolerance


def test_shutdown(setup_gpu: "tuple[NvidiaGPU, MagicMock | AsyncMock]"):
    gpu, mock_pynvml = setup_gpu
    gpu.shutdown()
    mock_pynvml.nvmlShutdown.assert_called_once()  # Ensure nvmlShutdown was called
