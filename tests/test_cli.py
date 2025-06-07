import pytest
from click.testing import CliRunner
from unittest.mock import patch
import re
from emt.cli import main, setup

@pytest.fixture
def runner():
    return CliRunner()

def test_main_help(runner):
    result = runner.invoke(main, ['--help'])
    assert result.exit_code == 0
    assert 'Usage:' in result.output

def test_main_execution(runner):
    with patch("emt.cli.setup") as mock_setup:
        result = runner.invoke(main, ['--interval', '5'])
        assert result.exit_code == 0
        mock_setup.assert_called_once()

def test_setup_service_not_enabled(runner):
    with patch("emt.cli._is_service_enabled", return_value=False) as mock_is_enabled, \
         patch("emt.cli._ensure_group") as mock_ensure_group, \
         patch("emt.cli._advertise_group_membership") as mock_advertise, \
         patch("emt.cli._install_systemd_unit") as mock_install_unit, \
         patch("emt.cli.logger") as mock_logger:

        result = runner.invoke(setup)

        assert result.exit_code == 0  # Ensure the command exits successfully
        mock_is_enabled.assert_called_once()
        mock_ensure_group.assert_called_once()
        mock_advertise.assert_called_once()
        mock_install_unit.assert_called_once()

        # Use regex to verify key phrases in log messages
        log_messages = [call[0][0] for call in mock_logger.info.call_args_list]
        assert any(re.search(r"installed and enabled", msg) for msg in log_messages)

def test_setup_service_already_enabled(runner):
    with patch("emt.cli._is_service_enabled", return_value=True) as mock_is_enabled, \
         patch("emt.cli.logger") as mock_logger:

        result = runner.invoke(setup)

        assert result.exit_code == 0  # Ensure the command exits successfully
        mock_is_enabled.assert_called_once()

        # Use regex to verify key phrases in log messages
        log_messages = [call[0][0] for call in mock_logger.info.call_args_list]
        assert any(re.search(r"already enabled", msg) for msg in log_messages)



if __name__ == "__main__":
    pytest.main([__file__])
