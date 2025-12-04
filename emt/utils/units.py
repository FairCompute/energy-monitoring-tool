"""
Unit conversion and validation utilities for the Energy Monitoring Tool (EMT).

This module provides utilities for converting between different units of measurement
and validating that configuration values use the correct units.
"""

import logging
from typing import Union, Dict, Any, Optional
from enum import Enum

logger = logging.getLogger(__name__)


class EnergyUnit(Enum):
    """Supported energy units."""

    JOULES = "Joules"
    KILOJOULES = "kJ"
    MICROJOULES = "μJ"
    MILLIJOULES = "mJ"
    WATTHOURS = "Wh"
    KILOWATTHOURS = "kWh"


class PowerUnit(Enum):
    """Supported power units."""

    WATTS = "Watts"
    KILOWATTS = "kW"
    MILLIWATTS = "mW"


# Conversion factors to base units (Joules, Watts)
ENERGY_CONVERSIONS = {
    EnergyUnit.JOULES: 1.0,
    EnergyUnit.KILOJOULES: 1000.0,
    EnergyUnit.MICROJOULES: 1e-6,
    EnergyUnit.MILLIJOULES: 1e-3,
    EnergyUnit.WATTHOURS: 3600.0,  # 1 Wh = 3600 J
    EnergyUnit.KILOWATTHOURS: 3600000.0,  # 1 kWh = 3.6 MJ
}

POWER_CONVERSIONS = {
    PowerUnit.WATTS: 1.0,
    PowerUnit.KILOWATTS: 1000.0,
    PowerUnit.MILLIWATTS: 1e-3,
}


class UnitConverter:
    """Utility class for unit conversions and validations."""

    @staticmethod
    def convert_energy(value: float, from_unit: str, to_unit: str) -> float:
        """
        Convert energy between different units.

        Args:
            value: The energy value to convert
            from_unit: Source unit (e.g., "kJ", "Joules")
            to_unit: Target unit (e.g., "Joules", "kWh")

        Returns:
            Converted energy value

        Raises:
            ValueError: If units are not supported
        """
        try:
            from_enum = EnergyUnit(from_unit)
            to_enum = EnergyUnit(to_unit)
        except ValueError as e:
            raise ValueError(f"Unsupported energy unit: {e}")

        # Convert to base unit (Joules) then to target unit
        joules = value * ENERGY_CONVERSIONS[from_enum]
        return joules / ENERGY_CONVERSIONS[to_enum]

    @staticmethod
    def convert_power(value: float, from_unit: str, to_unit: str) -> float:
        """
        Convert power between different units.

        Args:
            value: The power value to convert
            from_unit: Source unit (e.g., "kW", "Watts")
            to_unit: Target unit (e.g., "Watts", "mW")

        Returns:
            Converted power value

        Raises:
            ValueError: If units are not supported
        """
        try:
            from_enum = PowerUnit(from_unit)
            to_enum = PowerUnit(to_unit)
        except ValueError as e:
            raise ValueError(f"Unsupported power unit: {e}")

        # Convert to base unit (Watts) then to target unit
        watts = value * POWER_CONVERSIONS[from_enum]
        return watts / POWER_CONVERSIONS[to_enum]

    @staticmethod
    def normalize_sampling_interval_to_rate(
        sampling_interval: float | int,
    ) -> int:
        """
        Convert sampling interval (in seconds) to rate (measurements per second).

        Args:
            sampling_interval: Time between measurements in seconds

        Returns:
            Rate in Hz (measurements per second) as integer

        Raises:
            ValueError: If sampling_interval is <= 0
        """
        if sampling_interval <= 0:
            raise ValueError("Sampling interval must be positive")

        rate = 1.0 / sampling_interval
        return max(1, int(round(rate)))  # Ensure at least 1 Hz

    @staticmethod
    def validate_measurement_units(config: Dict[str, Any]) -> bool:
        """
        Validate that measurement units in config are supported.

        Args:
            config: Configuration dictionary

        Returns:
            True if all units are valid

        Raises:
            ValueError: If any unit is not supported
        """
        measurement_units = config.get("measurement_units", {})

        # Validate energy unit
        energy_unit = measurement_units.get("energy")
        if energy_unit:
            try:
                EnergyUnit(energy_unit)
            except ValueError:
                raise ValueError(f"Unsupported energy unit: {energy_unit}")

        # Validate power unit
        power_unit = measurement_units.get("power")
        if power_unit:
            try:
                PowerUnit(power_unit)
            except ValueError:
                raise ValueError(f"Unsupported power unit: {power_unit}")

        return True

    @staticmethod
    def validate_power_group_config(config: Dict[str, Any]) -> Dict[str, Any]:
        """
        Validate and normalize power group configuration.

        This function:
        1. Validates that rate values are positive integers
        2. Converts sampling_interval to rate if present
        3. Ensures rate is at least 1 Hz

        Args:
            config: Configuration dictionary

        Returns:
            Normalized configuration dictionary

        Raises:
            ValueError: If configuration values are invalid
        """
        normalized_config = config.copy()
        power_groups = normalized_config.get("power_groups", {})

        for group_name, group_config in power_groups.items():
            if not isinstance(group_config, dict):
                continue

            # Handle legacy sampling_interval parameter
            if "sampling_interval" in group_config:
                sampling_interval = group_config["sampling_interval"]
                if (
                    not isinstance(sampling_interval, (int, float))
                    or sampling_interval <= 0
                ):
                    raise ValueError(
                        f"Invalid sampling_interval for {group_name}: must be positive number"
                    )

                # Convert to rate and remove sampling_interval
                rate = UnitConverter.normalize_sampling_interval_to_rate(
                    sampling_interval
                )
                group_config["rate"] = rate
                del group_config["sampling_interval"]
                logger.info(
                    f"Converted {group_name} sampling_interval {sampling_interval}s to rate {rate}Hz"
                )

            # Validate rate parameter
            if "rate" in group_config:
                rate = group_config["rate"]
                if not isinstance(rate, int) or rate <= 0:
                    raise ValueError(
                        f"Invalid rate for {group_name}: must be positive integer (Hz)"
                    )
                group_config["rate"] = rate

        return normalized_config


# Public API
__all__ = [
    "EnergyUnit",
    "PowerUnit",
    "UnitConverter",
]
