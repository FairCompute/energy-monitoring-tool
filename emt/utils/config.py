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

# Public API
__all__ = [
    "load_config",
    "get_config",
    "get_config_value",
    "reload_config",
    "update_config",
    "save_config",
    "CONFIG",
]


# Default configuration path
_CONFIG_FILE_PATH = Path(__file__).parent / "config.json"

# Global configuration cache
_config_cache: Optional[Dict[str, Any]] = None

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
    global _config_cache

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

        # Cache the configuration
        _config_cache = config_data
        logger.debug(f"Configuration loaded from: {config_path}")

        return config_data

    except json.JSONDecodeError as e:
        logger.error(f"Invalid JSON in configuration file {config_path}: {e}")
        raise
    except Exception as e:
        logger.error(f"Error loading configuration from {config_path}: {e}")
        raise


def get_config(config_path: Optional[Union[str, Path]] = None) -> Dict[str, Any]:
    """
    Get the configuration dictionary. Loads from file if not already cached.

    Args:
        config_path: Path to the configuration file. If None, uses the default config.json.

    Returns:
        Dictionary containing the configuration data.
    """
    global _config_cache

    if _config_cache is None:
        _config_cache = load_config(config_path)

    return _config_cache


def get_config_value(
    key_path: str, default: Any = None, config_path: Optional[Union[str, Path]] = None
) -> Any:
    """
    Get a specific configuration value using a dot-separated key path.

    Args:
        key_path: Dot-separated path to the configuration value (e.g., "logger.level").
        default: Default value to return if the key path is not found.
        config_path: Path to the configuration file. If None, uses the cached config or default file.

    Returns:
        The configuration value or the default if not found.

    Examples:
        >>> get_config_value("logger.level")
        "INFO"
        >>> get_config_value("logger.format")
        "%(asctime)s - %(name)s - %(levelname)s - %(message)s"
        >>> get_config_value("nonexistent.key", "default_value")
        "default_value"
    """
    config = get_config(config_path)

    # Navigate through the nested dictionary using the key path
    keys = key_path.split(".")
    current_value = config

    try:
        for key in keys:
            current_value = current_value[key]
        return current_value
    except (KeyError, TypeError):
        logger.debug(
            f"Configuration key '{key_path}' not found, returning default: {default}"
        )
        return default


def reload_config(config_path: Optional[Union[str, Path]] = None) -> Dict[str, Any]:
    """
    Force reload the configuration from file, clearing the cache.

    Args:
        config_path: Path to the configuration file. If None, uses the default config.json.

    Returns:
        Dictionary containing the freshly loaded configuration data.
    """
    global _config_cache
    _config_cache = None  # Clear the cache
    return load_config(config_path)


def update_config(
    updates: Dict[str, Any], config_path: Optional[Union[str, Path]] = None
) -> None:
    """
    Update configuration values in memory. This does not persist changes to file.

    Args:
        updates: Dictionary of configuration updates to apply.
        config_path: Path to the configuration file. If None, uses the cached config or default file.

    Note:
        This only updates the in-memory configuration. To persist changes,
        use save_config() after calling this method.
    """
    config = get_config(config_path)

    def deep_update(target: Dict[str, Any], source: Dict[str, Any]) -> None:
        """Recursively update nested dictionaries."""
        for key, value in source.items():
            if (
                key in target
                and isinstance(target[key], dict)
                and isinstance(value, dict)
            ):
                deep_update(target[key], value)
            else:
                target[key] = value

    deep_update(config, updates)
    logger.debug(f"Configuration updated with: {updates}")


def save_config(
    config_path: Optional[Union[str, Path]] = None,
    config_data: Optional[Dict[str, Any]] = None,
) -> None:
    """
    Save configuration dictionary to JSON file.

    Args:
        config_path: Path to save the configuration file. If None, uses the default config.json.
        config_data: Configuration data to save. If None, uses the current cached configuration.

    Raises:
        ValueError: If no configuration data is available to save.
    """
    if config_path is None:
        config_path = _CONFIG_FILE_PATH
    else:
        config_path = Path(config_path)

    if config_data is None:
        config_data = get_config()

    if config_data is None:
        raise ValueError("No configuration data available to save")

    # Ensure the directory exists
    config_path.parent.mkdir(parents=True, exist_ok=True)

    try:
        with open(config_path, "w", encoding="utf-8") as config_file:
            json.dump(config_data, config_file, indent=4, ensure_ascii=False)

        logger.info(f"Configuration saved to: {config_path}")

    except Exception as e:
        logger.error(f"Error saving configuration to {config_path}: {e}")
        raise


# Convenience function to get the configuration as a module-level variable
CONFIG = get_config()


if __name__ == "__main__":
    # Example usage and testing
    print("EMT Configuration:")
    print(json.dumps(CONFIG, indent=2))

    print("\nLogger configuration:")
    print(f"Level: {get_config_value('logger.level')}")
    print(f"Format: {get_config_value('logger.format')}")
    print(f"Log Directory: {get_config_value('logger.log_directory')}")

    print("\nTesting default values:")
    print(f"Nonexistent key: {get_config_value('nonexistent.key', 'DEFAULT_VALUE')}")
