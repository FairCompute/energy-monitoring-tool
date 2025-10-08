"""
Tests for the units module.
"""

import pytest
from emt.utils.units import UnitConverter, EnergyUnit, PowerUnit


class TestUnitConverter:
    """Test the UnitConverter class."""

    def test_energy_conversions(self):
        """Test energy unit conversions."""
        converter = UnitConverter()

        # Test Joules to kJ
        assert abs(converter.convert_energy(1000, "Joules", "kJ") - 1.0) < 1e-9

        # Test kWh to Joules
        assert abs(converter.convert_energy(1, "kWh", "Joules") - 3600000.0) < 1e-9

        # Test microjoules to Joules
        assert abs(converter.convert_energy(1000000, "μJ", "Joules") - 1.0) < 1e-9

        # Test millijoules to Joules
        assert abs(converter.convert_energy(1000, "mJ", "Joules") - 1.0) < 1e-9

        # Test Wh to Joules
        assert abs(converter.convert_energy(1, "Wh", "Joules") - 3600.0) < 1e-9

    def test_power_conversions(self):
        """Test power unit conversions."""
        converter = UnitConverter()

        # Test Watts to kW
        assert abs(converter.convert_power(1000, "Watts", "kW") - 1.0) < 1e-9

        # Test kW to Watts
        assert abs(converter.convert_power(1, "kW", "Watts") - 1000.0) < 1e-9

        # Test mW to Watts
        assert abs(converter.convert_power(1000, "mW", "Watts") - 1.0) < 1e-9

    def test_sampling_interval_to_rate(self):
        """Test sampling interval to rate conversion."""
        converter = UnitConverter()

        # Test basic conversions
        assert converter.normalize_sampling_interval_to_rate(1.0) == 1
        assert converter.normalize_sampling_interval_to_rate(0.5) == 2
        assert converter.normalize_sampling_interval_to_rate(0.1) == 10
        assert converter.normalize_sampling_interval_to_rate(2.0) == 1  # minimum 1 Hz

        # Test minimum rate constraint
        assert converter.normalize_sampling_interval_to_rate(10.0) == 1

        # Test rounding
        assert (
            converter.normalize_sampling_interval_to_rate(0.33) == 3
        )  # 1/0.33 ≈ 3.03 -> 3

    def test_invalid_units(self):
        """Test handling of invalid units."""
        converter = UnitConverter()

        # Test invalid energy unit
        with pytest.raises(ValueError, match="Unsupported energy unit"):
            converter.convert_energy(100, "invalid_unit", "Joules")

        # Test invalid power unit
        with pytest.raises(ValueError, match="Unsupported power unit"):
            converter.convert_power(100, "Watts", "invalid_unit")

    def test_invalid_sampling_interval(self):
        """Test invalid sampling interval values."""
        converter = UnitConverter()

        # Test zero sampling interval
        with pytest.raises(ValueError, match="Sampling interval must be positive"):
            converter.normalize_sampling_interval_to_rate(0)

        # Test negative sampling interval
        with pytest.raises(ValueError, match="Sampling interval must be positive"):
            converter.normalize_sampling_interval_to_rate(-1)

    def test_validate_measurement_units(self):
        """Test measurement unit validation."""
        converter = UnitConverter()

        # Valid configuration
        valid_config = {"measurement_units": {"energy": "Joules", "power": "Watts"}}
        assert converter.validate_measurement_units(valid_config) is True

        # Invalid energy unit
        invalid_config = {
            "measurement_units": {"energy": "invalid_unit", "power": "Watts"}
        }
        with pytest.raises(ValueError, match="Unsupported energy unit"):
            converter.validate_measurement_units(invalid_config)

        # Missing measurement_units section should be valid
        empty_config = {}
        assert converter.validate_measurement_units(empty_config) is True

    def test_validate_power_group_config(self):
        """Test power group configuration validation."""
        converter = UnitConverter()

        # Test conversion from sampling_interval to rate
        config_with_sampling = {
            "power_groups": {
                "rapl": {"sampling_interval": 1.0},
                "gpu": {"sampling_interval": 0.5},
            }
        }

        normalized = converter.validate_power_group_config(config_with_sampling)

        # Check that sampling_interval was converted to rate
        assert "rate" in normalized["power_groups"]["rapl"]
        assert "sampling_interval" not in normalized["power_groups"]["rapl"]
        assert normalized["power_groups"]["rapl"]["rate"] == 1

        assert "rate" in normalized["power_groups"]["gpu"]
        assert "sampling_interval" not in normalized["power_groups"]["gpu"]
        assert normalized["power_groups"]["gpu"]["rate"] == 2

        # Test valid rate configuration
        config_with_rate = {"power_groups": {"rapl": {"rate": 5}}}

        normalized = converter.validate_power_group_config(config_with_rate)
        assert normalized["power_groups"]["rapl"]["rate"] == 5

        # Test invalid rate (not integer)
        config_invalid_rate = {"power_groups": {"rapl": {"rate": 1.5}}}

        with pytest.raises(ValueError, match="Invalid rate for rapl"):
            converter.validate_power_group_config(config_invalid_rate)

        # Test invalid sampling_interval (negative)
        config_invalid_sampling = {"power_groups": {"rapl": {"sampling_interval": -1}}}

        with pytest.raises(ValueError, match="Invalid sampling_interval for rapl"):
            converter.validate_power_group_config(config_invalid_sampling)


class TestUnitEnums:
    """Test the unit enumeration classes."""

    def test_energy_units(self):
        """Test EnergyUnit enum."""
        assert EnergyUnit.JOULES.value == "Joules"
        assert EnergyUnit.KILOJOULES.value == "kJ"
        assert EnergyUnit.MICROJOULES.value == "μJ"
        assert EnergyUnit.KILOWATTHOURS.value == "kWh"

    def test_power_units(self):
        """Test PowerUnit enum."""
        assert PowerUnit.WATTS.value == "Watts"
        assert PowerUnit.KILOWATTS.value == "kW"
        assert PowerUnit.MILLIWATTS.value == "mW"
