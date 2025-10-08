"""
Configuration module for the Energy Monitoring Tool (EMT).

This module loads configuration from config.json and provides it as a Python dictionary.
It also includes utilities for accessing nested configuration values safely.
"""

import json
import os
import logging
from pathlib import Path
from typing import Dict, Any, Optional, Union

# Import unit validation utilities
try:
    from .units import UnitConverter
except ImportError:
    # When running as script directly
    from units import UnitConverter

# Public API
__all__ = [
    "load_config",
    "validate_config",
]


# Default configuration path
_CONFIG_FILE_PATH = Path(__file__).parent.parent / "config.json"

logger = logging.getLogger(__name__)


def load_config(config_path: Optional[Union[str, Path]] = None) -> Dict[str, Any]:
    """
    Load configuration from JSON file.

    Args:
        config_path: Path to the configuration file. If None, uses the default config.json
                    in the same directory as this module.

    Returns:
        Dictionary containing the configuration data.

    Raises:
        FileNotFoundError: If the configuration file doesn't exist.
        json.JSONDecodeError: If the configuration file contains invalid JSON.
    """
    if config_path is None:
        config_path = _CONFIG_FILE_PATH
    else:
        config_path = Path(config_path)

    # Check if file exists
    if not config_path.exists():
        raise FileNotFoundError(f"Configuration file not found: {config_path}")

    try:
        with open(config_path, "r", encoding="utf-8") as config_file:
            config_data = json.load(config_file)

        logger.debug(f"Configuration loaded from: {config_path}")

        return config_data

    except json.JSONDecodeError as e:
        logger.error(f"Invalid JSON in configuration file {config_path}: {e}")
        raise
    except Exception as e:
        logger.error(f"Error loading configuration from {config_path}: {e}")
        raise


def validate_config(config_data: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
    """
    Validate and normalize configuration data.

    This function:
    1. Validates measurement units are supported
    2. Converts sampling_interval to rate in power groups
    3. Validates rate values are positive integers

    Args:
        config_data: Configuration data to validate. If None, uses current cached config.

    Returns:
        Validated and normalized configuration dictionary

    Raises:
        ValueError: If configuration contains invalid values
    """
    if config_data is None:
        config_data = load_config()

    # Create a copy to avoid modifying the original
    validated_config = config_data.copy()

    try:
        # Validate measurement units
        UnitConverter.validate_measurement_units(validated_config)

        # Validate and normalize power group configuration
        validated_config = UnitConverter.validate_power_group_config(validated_config)

        logger.debug("Configuration validation completed successfully")
        return validated_config

    except Exception as e:
        logger.error(f"Configuration validation failed: {e}")
        raise


if __name__ == "__main__":
    # Example usage and testing
    config = load_config()
    print("EMT Configuration:")
    print(json.dumps(config, indent=2))

    # Test validation
    try:
        validated_config = validate_config(config)
        print("\nValidation successful!")
    except Exception as e:
        print(f"\nValidation failed: {e}")
