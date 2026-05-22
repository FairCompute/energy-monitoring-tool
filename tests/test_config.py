import json

import pytest

from emt.utils.config import load_config, validate_config


def test_load_config_reads_explicit_json_file(tmp_path):
    config_path = tmp_path / "config.json"
    config_path.write_text(
        json.dumps(
            {
                "measurement_units": {"energy": "Joules", "power": "Watts"},
                "power_groups": {"rapl": {"rate": 1}},
            }
        ),
        encoding="utf-8",
    )

    assert load_config(config_path) == {
        "measurement_units": {"energy": "Joules", "power": "Watts"},
        "power_groups": {"rapl": {"rate": 1}},
    }


def test_load_config_raises_for_missing_file(tmp_path):
    with pytest.raises(FileNotFoundError, match="Configuration file not found"):
        load_config(tmp_path / "missing.json")


def test_load_config_raises_for_invalid_json(tmp_path):
    config_path = tmp_path / "config.json"
    config_path.write_text("{not-json", encoding="utf-8")

    with pytest.raises(json.JSONDecodeError):
        load_config(config_path)


def test_validate_config_loads_default_when_config_not_supplied(monkeypatch):
    loaded_config = {
        "measurement_units": {"energy": "Joules", "power": "Watts"},
        "power_groups": {"rapl": {"sampling_interval": 0.5}},
    }

    monkeypatch.setattr("emt.utils.config.load_config", lambda: loaded_config)

    validated = validate_config()

    assert validated["power_groups"]["rapl"] == {"rate": 2}


def test_validate_config_rejects_invalid_units():
    with pytest.raises(ValueError, match="Unsupported energy unit"):
        validate_config(
            {
                "measurement_units": {"energy": "bogus", "power": "Watts"},
                "power_groups": {"rapl": {"rate": 1}},
            }
        )


def test_validate_config_rejects_invalid_power_group_rate():
    with pytest.raises(ValueError, match="Invalid rate for rapl"):
        validate_config(
            {
                "measurement_units": {"energy": "Joules", "power": "Watts"},
                "power_groups": {"rapl": {"rate": 0}},
            }
        )
