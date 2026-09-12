"""Persistent application settings.

QSettings is used instead of a custom config file so the application follows
the native Linux configuration location and does not need root privileges.
"""

from __future__ import annotations

import json
from typing import Any

from PyQt5.QtCore import QSettings


class ConfigManager:
    """Read and write small pieces of user configuration."""

    def __init__(self) -> None:
        self.settings = QSettings("UARTAssistant", "UARTAssistant")

    def load_macros(self) -> list[dict[str, Any]]:
        """Return saved macro definitions, falling back to useful defaults."""
        raw = self.settings.value("macros", "")
        if raw:
            try:
                data = json.loads(str(raw))
                if isinstance(data, list):
                    return [item for item in data if isinstance(item, dict)]
            except (TypeError, ValueError):
                pass
        return [
            {"name": "AT", "command": "AT", "hex": False},
            {"name": "AT+RST", "command": "AT+RST", "hex": False},
            {"name": "Help", "command": "help", "hex": False},
        ]

    def save_macros(self, macros: list[dict[str, Any]]) -> None:
        """Persist macro definitions as JSON."""
        self.settings.setValue("macros", json.dumps(macros, ensure_ascii=False))
        self.settings.sync()

    def load_value(self, key: str, default: Any = None, type_: type | None = None) -> Any:
        if type_ is not None:
            return self.settings.value(key, default, type_)
        return self.settings.value(key, default)

    def save_value(self, key: str, value: Any) -> None:
        self.settings.setValue(key, value)
        self.settings.sync()
