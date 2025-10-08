import pytest
from click.testing import CliRunner
from unittest.mock import patch
import re
from emt.cfgup import main, setup


@pytest.fixture
def runner():
    return CliRunner()


def test_main_help(runner):
    result = runner.invoke(main, ["--help"])
    assert result.exit_code == 0
    assert "Usage:" in result.output


def test_main_execution(runner):
    with patch("emt.cfgup.setup") as mock_setup:
        result = runner.invoke(main, [])
        assert result.exit_code == 0
        mock_setup.assert_called_once()


def test_setup_service_not_enabled(runner):
    with (
        patch("emt.cfgup._is_service_enabled", side_effect=[False, True]) as mock_is_enabled,
        patch("emt.cfgup._is_service_loaded_properly", side_effect=[False, True]),
        patch("emt.cfgup._is_service_active", side_effect=[False, True]),
        patch("emt.cfgup._ensure_group") as mock_ensure_group,
        patch("emt.cfgup._advertise_group_membership") as mock_advertise,
        patch("emt.cfgup._install_systemd_unit") as mock_install_unit,
        patch("emt.cfgup.subprocess.run"),
        patch("emt.cfgup.logger") as mock_logger,
    ):

        result = runner.invoke(setup)

        assert result.exit_code == 0  # Ensure the command exits successfully
        assert mock_is_enabled.call_count == 2  # Called once initially and once for verification
        mock_ensure_group.assert_called_once()
        mock_advertise.assert_called_once()
        mock_install_unit.assert_called_once()

        # Use regex to verify key phrases in log messages
        log_messages = [call[0][0] for call in mock_logger.info.call_args_list]
        assert any(re.search(r"installed and enabled", msg) for msg in log_messages)


def test_setup_service_already_enabled(runner):
    with (
        patch("emt.cfgup._is_service_enabled", return_value=True) as mock_is_enabled,
        patch("emt.cfgup._is_service_loaded_properly", return_value=True),
        patch("emt.cfgup._is_service_active", return_value=True),
        patch("emt.cfgup.logger") as mock_logger,
    ):

        result = runner.invoke(setup)

        assert result.exit_code == 0  # Ensure the command exits successfully
        mock_is_enabled.assert_called_once()

        # Use regex to verify key phrases in log messages
        log_messages = [call[0][0] for call in mock_logger.info.call_args_list]
        assert any(re.search(r"already properly configured", msg) for msg in log_messages)


if __name__ == "__main__":
    pytest.main([__file__])
