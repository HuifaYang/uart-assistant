"""Low-cost Linux serial-device hotplug polling."""

from __future__ import annotations

from PyQt5.QtCore import QObject, QTimer, pyqtSignal, pyqtSlot

from model.serial_port import DeviceInfo, scan_serial_devices


class HotplugMonitor(QObject):
    """Poll device metadata in a worker thread and emit only on changes."""

    devices_changed = pyqtSignal(object)
    scan_requested = pyqtSignal()
    stop_requested = pyqtSignal()

    def __init__(self, interval_ms: int = 1500) -> None:
        super().__init__()
        self.interval_ms = interval_ms
        self.timer: QTimer | None = None
        self._snapshot: tuple[tuple[str, str], ...] = ()
        self.scan_requested.connect(self.scan)
        self.stop_requested.connect(self.stop)

    @pyqtSlot()
    def start(self) -> None:
        self.timer = QTimer(self)
        self.timer.setInterval(self.interval_ms)
        self.timer.timeout.connect(self.scan)
        self.timer.start()
        self.scan()

    @pyqtSlot()
    def scan(self) -> None:
        devices = scan_serial_devices()
        snapshot = tuple((item.path, item.display_name) for item in devices)
        if snapshot != self._snapshot:
            self._snapshot = snapshot
            self.devices_changed.emit(devices)

    @pyqtSlot()
    def stop(self) -> None:
        if self.timer is not None:
            self.timer.stop()
        from PyQt5.QtCore import QThread
        QThread.currentThread().quit()
