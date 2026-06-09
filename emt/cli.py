"""Installed console entry point for the EMT Rust CLI."""

from __future__ import annotations

import sys


def main() -> int:
    """Run the native Rust CLI exposed by the bundled PyO3 extension."""
    try:
        from emt._rust import cli_main
    except ImportError as exc:
        raise RuntimeError(
            "The installed emt command requires the bundled Rust extension "
            "`emt._rust`. Reinstall EMT from a wheel or from source with maturin."
        ) from exc

    return int(cli_main(sys.argv))
