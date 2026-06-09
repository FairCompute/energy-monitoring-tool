import sys
import types
import builtins

import pytest

from emt import cli


def test_cli_entrypoint_delegates_to_rust_module(monkeypatch):
    calls = []

    def fake_cli_main(argv):
        calls.append(argv)
        return 7

    monkeypatch.setitem(
        sys.modules,
        "emt._rust",
        types.SimpleNamespace(cli_main=fake_cli_main),
    )
    monkeypatch.setattr(sys, "argv", ["emt", "--help"])

    assert cli.main() == 7
    assert calls == [["emt", "--help"]]


def test_cli_entrypoint_reports_missing_rust_extension(monkeypatch):
    original_import = builtins.__import__

    def fake_import(name, *args, **kwargs):
        if name == "emt._rust":
            raise ImportError("missing rust extension")
        return original_import(name, *args, **kwargs)

    monkeypatch.setattr(builtins, "__import__", fake_import)

    with pytest.raises(RuntimeError, match="requires the bundled Rust extension"):
        cli.main()
