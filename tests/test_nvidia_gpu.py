import pytest
from emt.power_groups import DeltaCalculator


class TestDeltaCalculator:
    def test_initialization(self):
        # Test default initialization
        calculator = DeltaCalculator()
        assert calculator._init_energy == 0.0

        # Test initialization with a specific value
        calculator = DeltaCalculator(100.0)
        assert calculator._init_energy == 100.0

    def test_call_method(self):
        calculator = DeltaCalculator()

        # First call should calculate difference from initial energy (0.0)
        result = calculator(5000.0)  # 5000 J input
        assert result == 5.0  # (5000 - 0) / 1000

        # Second call should calculate difference from last input (5000.0)
        result = calculator(7000.0)  # 7000 J input
        assert result == 2.0  # (7000 - 5000) / 1000

        # Third call with no previous input, should work from last input
        result = calculator(7000.0)  # 7000 J input
        assert result == 0.0  # (7000 - 7000) / 1000

        # Ensure that the internal state updates correctly
        assert calculator._init_energy == 7000.0

    def test_negative_energy(self):
        calculator = DeltaCalculator(5000.0)

        # Calculate energy with a lower current energy
        result = calculator(3000.0)  # 3000 J input
        assert result == -2.0  # (3000 - 5000) / 1000

    def test_float_precision(self):
        calculator = DeltaCalculator(1.5)
        result = calculator(2.5)  # 2.5 J input
        assert result == 0.001  # (2.5 - 1.5) / 1000
