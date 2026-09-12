"""Serial-port model and Linux device discovery.

The SerialPort class deliberately contains no Qt code.  It is owned by
SerialWorker and is therefore only accessed from the worker thread.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
import re

import serial
from serial.tools import list_ports


@dataclass
class DeviceInfo:
    """A serial device suitable for presentation in a combo box."""

    path: str
    description: str = "未知设备"
    vid: int | None = None
    pid: int | None = None
    manufacturer: str = ""
    serial_number: str = ""
    aliases: list[str] = field(default_factory=list)

    @property
    def usb_id(self) -> str:
        if self.vid is None or self.pid is None:
            return "USB --/--"
        return f"USB {self.vid:04X}:{self.pid:04X}"

    @property
    def display_name(self) -> str:
        # Keep the combo-box row compact.  Stable aliases are attached as a
        # tooltip by the view instead of making the popup extremely wide.
        return f"{self.path} - {self.description} ({self.usb_id})"


@dataclass
class PortSettings:
    """Values accepted by pyserial, represented in UI-friendly form."""

    port: str
    baudrate: int = 115200
    bytesize: int = 8
    stopbits: float = serial.STOPBITS_ONE
    parity: str = serial.PARITY_NONE
    flow: str = "none"

    @classmethod
    def from_dict(cls, values: dict) -> "PortSettings":
        return cls(
            port=str(values["port"]),
            baudrate=int(values.get("baudrate", 115200)),
            bytesize=int(values.get("bytesize", 8)),
            stopbits=float(values.get("stopbits", 1)),
            parity=str(values.get("parity", "N")),
            flow=str(values.get("flow", "none")),
        )


def _linux_aliases() -> dict[str, list[str]]:
    """Resolve stable /dev/serial/by-id and by-path aliases on Linux."""
    aliases: dict[str, list[str]] = {}
    for root in (Path("/dev/serial/by-id"), Path("/dev/serial/by-path")):
        if not root.exists():
            continue
        try:
            entries = root.iterdir()
        except OSError:
            continue
        for entry in entries:
            try:
                target = str(entry.resolve())
            except OSError:
                continue
            aliases.setdefault(target, []).append(str(entry))
    return aliases


def scan_serial_devices() -> list[DeviceInfo]:
    """Scan useful Linux serial devices and merge aliases by physical target.

    ``include_links=True`` may return both ``/dev/ttyUSB0`` and one or more
    ``/dev/serial/by-*`` links.  They are the same hardware, so presenting
    each path as a separate row is misleading.  Grouping by the resolved
    target also prevents the alias fallback below from creating duplicates.
    """
    alias_map = _linux_aliases()
    result: dict[str, DeviceInfo] = {}
    # The default list is for external development adapters.  Linux often
    # exposes many internal 16550/SoC UARTs as ttyS*, ttyAMA*, ttyTHS* and
    # similar nodes; most are not connected and make the device popup noisy.
    # They remain usable through manual entry in the editable combo box.
    usb_name = re.compile(r"^(?:ttyUSB\d+|ttyACM\d+|rfcomm\d+)$")

    def target_path(path: str) -> str:
        # The reverse lookup also works when a udev link has disappeared
        # between enumeration and resolution, or when a backend reports the
        # alias text without a real Path object.
        for target, aliases in alias_map.items():
            if path in aliases:
                return target
        try:
            return str(Path(path).resolve()) if Path(path).is_symlink() else path
        except OSError:
            return path

    def is_useful(path: str, target: str, item) -> bool:
        return (
            usb_name.fullmatch(Path(target).name) is not None
            # A real USB adapter must provide both fields.  Some platform
            # UART backends expose a stray VID without a PID and would
            # otherwise leak ttyS*/ttyAMA* entries into the USB list.
            or (item.vid is not None and item.pid is not None)
        )

    try:
        ports = list_ports.comports(include_links=True)
    except (OSError, IOError):
        ports = []

    for item in ports:
        path = str(item.device)
        resolved_path = target_path(path)
        if not is_useful(path, resolved_path, item):
            continue
        current = result.get(resolved_path)
        if current is None:
            result[resolved_path] = DeviceInfo(
                path=resolved_path,
                description=item.description or "未知设备",
                vid=item.vid,
                pid=item.pid,
                manufacturer=item.manufacturer or "",
                serial_number=item.serial_number or "",
                aliases=[],
            )
        else:
            # Prefer the entry carrying actual USB metadata when pyserial
            # reports an alias first without VID/PID information.
            if current.vid is None and item.vid is not None:
                current.description = item.description or current.description
                current.vid = item.vid
                current.pid = item.pid
                current.manufacturer = item.manufacturer or current.manufacturer
                current.serial_number = item.serial_number or current.serial_number

    # Some Linux aliases are not returned by an installed udev backend.
    for target, aliases in alias_map.items():
        if usb_name.fullmatch(Path(target).name) and target not in result and Path(target).exists():
            result[target] = DeviceInfo(
                path=target,
                description="串口设备",
                aliases=[],
            )

    # Keep the real kernel path as the open path.  Stable aliases are shown
    # only as metadata: this keeps the combo-box text short and ensures that
    # hotplug refreshes do not lose the selected item when udev renames or
    # reorders symlinks.
    for target, device in result.items():
        aliases = sorted(set(alias_map.get(target, [])))
        device.path = target
        device.aliases = [item for item in aliases if item != target]

    return sorted(result.values(), key=lambda item: item.path)


class SerialPort:
    """Small blocking pyserial wrapper, used only inside SerialWorker."""

    def __init__(self) -> None:
        self.serial: serial.Serial | None = None

    @property
    def is_open(self) -> bool:
        return bool(self.serial and self.serial.is_open)

    def open(self, settings: PortSettings) -> None:
        """Open a port and translate the UI flow-control setting."""
        if self.is_open:
            self.close()
        kwargs = {
            "port": settings.port,
            "baudrate": settings.baudrate,
            "bytesize": settings.bytesize,
            "stopbits": settings.stopbits,
            "parity": settings.parity,
            "timeout": 0.05,
            "write_timeout": 1.0,
        }
        if settings.flow == "hardware":
            kwargs.update(rtscts=True, dsrdtr=True)
        elif settings.flow == "software":
            kwargs.update(xonxoff=True)
        else:
            kwargs.update(rtscts=False, dsrdtr=False, xonxoff=False)
        self.serial = serial.Serial(**kwargs)

    def close(self) -> None:
        if self.serial is not None:
            self.serial.close()
        self.serial = None

    def read_available(self) -> bytes:
        if not self.is_open or self.serial is None:
            return b""
        waiting = self.serial.in_waiting
        return self.serial.read(waiting or 1) if waiting else b""

    def read(self, size: int = 1) -> bytes:
        if not self.is_open or self.serial is None:
            return b""
        return self.serial.read(size)

    def write(self, data: bytes) -> int:
        if not self.is_open or self.serial is None:
            raise serial.SerialException("Serial port is not open")
        return self.serial.write(data)

    def flush_input(self) -> None:
        if self.serial is not None and self.serial.is_open:
            self.serial.reset_input_buffer()

    def set_dtr(self, value: bool) -> None:
        if self.serial is None or not self.serial.is_open:
            raise serial.SerialException("Serial port is not open")
        self.serial.dtr = value

    def set_rts(self, value: bool) -> None:
        if self.serial is None or not self.serial.is_open:
            raise serial.SerialException("Serial port is not open")
        self.serial.rts = value

    def line_states(self) -> tuple[bool, bool]:
        if self.serial is None or not self.serial.is_open:
            return False, False
        return bool(self.serial.dtr), bool(self.serial.rts)
