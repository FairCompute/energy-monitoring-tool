#!/usr/bin/env python3
"""
Example usage of the EMT config module.
This example shows how to use the configuration module in your EMT applications.
"""

import emt


def main():
    print("=== EMT Configuration Usage Examples ===\n")

    # 1. Get the entire configuration
    print("1. Getting entire configuration:")
    config = emt.config.get_config()
    print(f"   Full config: {config}")
    print()

    # 2. Get specific configuration values
    print("2. Getting specific configuration values:")
    logger_level = emt.config.get_config_value("logger.level")
    logger_format = emt.config.get_config_value("logger.format")
    log_directory = emt.config.get_config_value("logger.log_directory")

    print(f"   Logger level: {logger_level}")
    print(f"   Logger format: {logger_format}")
    print(f"   Log directory: {log_directory}")
    print()

    # 3. Get configuration value with default
    print("3. Getting configuration value with default:")
    missing_value = emt.config.get_config_value("missing.key", "DEFAULT_VALUE")
    print(f"   Missing key with default: {missing_value}")
    print()

    # 4. Update configuration in memory
    print("4. Updating configuration in memory:")
    print(f"   Original logger level: {emt.config.get_config_value('logger.level')}")

    emt.config.update_config(
        {"logger": {"level": "DEBUG"}, "new_section": {"new_key": "new_value"}}
    )

    print(f"   Updated logger level: {emt.config.get_config_value('logger.level')}")
    print(
        f"   New configuration value: {emt.config.get_config_value('new_section.new_key')}"
    )
    print()

    # 5. Show that changes are in memory only
    print("5. Reloading config to show changes are in memory only:")
    emt.config.reload_config()
    print(
        f"   After reload, logger level: {emt.config.get_config_value('logger.level')}"
    )
    print(
        f"   After reload, new section exists: {emt.config.get_config_value('new_section.new_key', 'NOT_FOUND')}"
    )


if __name__ == "__main__":
    main()
