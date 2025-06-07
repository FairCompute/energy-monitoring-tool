import click
import grp
import logging
import subprocess
import sys
from pathlib import Path

from emt.utils.logger import setup_logger


logger = logging.getLogger("emt.cli")
setup_logger(logger)

# Optionally add stdout handler
def add_stdout_handler():
    stdout_handler = logging.StreamHandler(sys.stdout)
    stdout_handler.setLevel(logging.DEBUG)
    formatter = logging.Formatter('%(asctime)s - %(name)s - %(levelname)s - %(message)s')
    stdout_handler.setFormatter(formatter)
    logger.addHandler(stdout_handler)

# Call this function if stdout logging is needed
add_stdout_handler()

_GROUP_NAME = "powercap"


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
        "Then log out and log back in, or run 'newgrp {0}' for the change to take effect.".format(
            group_name
        )
    )


def _install_systemd_unit(destination="/etc/systemd/system/energy_access.service"):
    service_src = Path(__file__).parent.parent / "assets" / "energy_access.service"
    service_dst = Path(destination)
    logger.info(f"Installing systemd unit to {service_dst}...")
    subprocess.run(["sudo", "cp", str(service_src), str(service_dst)], check=True)
    subprocess.run(["sudo", "systemctl", "daemon-reexec"], check=True)
    subprocess.run(
        ["sudo", "systemctl", "enable", "--now", "energy_access.service"], check=True
    )


def _is_service_enabled(service="energy_access.service"):
    result = subprocess.run(
        ["systemctl", "is-enabled", service],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    return result.stdout.strip() == "enabled"


@click.command()
def setup() -> bool:
    if not _is_service_enabled():
        _ensure_group()
        _advertise_group_membership()
        try:
            _install_systemd_unit()
        except subprocess.CalledProcessError as e:
            logger.error(f"Failed to install systemd unit: {e}")
            return False
        logger.info("Service installed and enabled successfully.")
    else:
        logger.info("Service is already enabled. No action needed!.")
    return True


@click.command()
@click.option(
    "--interval",
    default=1,
    type=int,
    help="Interval in seconds for the collector to run. Default is 1 second.",
)
def main(interval: int):
    logger.info(f"Starting energy monitoring collector with interval {interval} seconds...")
    setup()

if __name__ == "__main__":
    main()
