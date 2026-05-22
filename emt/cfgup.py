import click
import grp
import logging
import subprocess
import sys
from pathlib import Path

logger = logging.getLogger("emt.cfgup")

SYSTEMD_ENERGY_ACCESS_UNIT = "energy_access.service"


# Optionally add stdout handler
def add_stdout_handler():
    stdout_handler = logging.StreamHandler(sys.stdout)
    stdout_handler.setLevel(logging.DEBUG)
    formatter = logging.Formatter(
        "%(asctime)s - %(name)s - %(levelname)s - %(message)s"
    )
    stdout_handler.setFormatter(formatter)
    logger.addHandler(stdout_handler)


# Call this function if stdout logging is needed
add_stdout_handler()

_GROUP_NAME = "powercap"
SYSTEMD_UNIT_DESTINATION = f"/etc/systemd/system/{SYSTEMD_ENERGY_ACCESS_UNIT}"


def _packaged_systemd_unit_path() -> Path:
    return Path(__file__).parent.parent / "assets" / SYSTEMD_ENERGY_ACCESS_UNIT


def _ensure_group(group_name=_GROUP_NAME):
    try:
        grp.getgrnam(group_name)
        logger.info(f"Group '{group_name}' already exists.")
    except KeyError:
        logger.info(f"Creating group '{group_name}'...")

        subprocess.run(["sudo", "groupadd", group_name], check=True)


def _advertise_group_membership(group_name=_GROUP_NAME):
    logger.info(
        f"To access energy monitoring as a non-root user, add yourself to the '{group_name}' group:\n"
        f"  sudo usermod -aG {group_name} $USER\n"
        "Then log out, or run 'newgrp {0}' for the change to take effect.".format(
            group_name
        )
    )


def _install_systemd_unit(destination=SYSTEMD_UNIT_DESTINATION):
    service_src = _packaged_systemd_unit_path()
    service_dst = Path(destination)
    logger.info(f"Installing systemd unit to {service_dst}...")
    subprocess.run(["sudo", "cp", str(service_src), str(service_dst)], check=True)
    subprocess.run(["sudo", "systemctl", "daemon-reload"], check=True)
    subprocess.run(
        ["sudo", "systemctl", "enable", SYSTEMD_ENERGY_ACCESS_UNIT], check=True
    )
    subprocess.run(
        ["sudo", "systemctl", "restart", SYSTEMD_ENERGY_ACCESS_UNIT], check=True
    )


def _is_service_enabled(service=SYSTEMD_ENERGY_ACCESS_UNIT):
    result = subprocess.run(
        ["systemctl", "is-enabled", service],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    return result.stdout.strip() == "enabled"


def _is_service_active(service=SYSTEMD_ENERGY_ACCESS_UNIT):
    result = subprocess.run(
        ["systemctl", "is-active", service],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    return result.stdout.strip() == "active"


def _is_service_loaded_properly(service=SYSTEMD_ENERGY_ACCESS_UNIT):
    result = subprocess.run(
        ["systemctl", "show", "-p", "LoadState", service],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    return "LoadState=loaded" in result.stdout


def _installed_unit_matches_package(
    destination=SYSTEMD_UNIT_DESTINATION,
) -> bool:
    service_dst = Path(destination)
    try:
        return service_dst.read_text() == _packaged_systemd_unit_path().read_text()
    except OSError:
        return False


@click.command()
def setup() -> bool:
    # Check if service exists and is properly loaded
    service_loaded = _is_service_loaded_properly()
    service_enabled = _is_service_enabled()
    service_active = _is_service_active()
    unit_current = _installed_unit_matches_package()

    logger.info(
        f"Service status - Loaded: {service_loaded}, Enabled: {service_enabled}, Active: {service_active}, Unit current: {unit_current}"
    )

    if (
        not service_loaded
        or not service_enabled
        or not service_active
        or not unit_current
    ):
        _ensure_group()
        _advertise_group_membership()
        try:
            # Stop and disable existing service if it exists but has issues
            if not service_loaded or not unit_current:
                logger.info("Service unit needs reinstalling...")
                subprocess.run(
                    ["sudo", "systemctl", "stop", SYSTEMD_ENERGY_ACCESS_UNIT],
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                )

            _install_systemd_unit()

            # Verify the service is now working
            if (
                _is_service_loaded_properly()
                and _is_service_enabled()
                and _is_service_active()
                and _installed_unit_matches_package()
            ):
                logger.info("Service installed and enabled successfully.")
            else:
                logger.error(
                    "Service installation completed but service is not working properly."
                )
                return False

        except subprocess.CalledProcessError as e:
            logger.error(f"Failed to install systemd unit: {e}")
            return False
    else:
        logger.info(
            "Service is already properly configured and running. No action needed!"
        )
    return True


@click.command()
# @click.option(
#     "--interval",
#     default=1,
#     type=int,
#     help="Interval in seconds for the collector to run. Default is 1 second.",
# )
def main():
    logger.info("Setting up EMT...")
    setup()


if __name__ == "__main__":
    main()
