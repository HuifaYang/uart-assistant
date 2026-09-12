"""Background QObject that owns every live serial connection."""

from __future__ import annotations

import threading

from PyQt5.QtCore import QObject, QTimer, Qt, pyqtSignal, pyqtSlot

from model.serial_port import PortSettings, SerialPort
from model.xmodem import XModemError, XModemSender


class SerialWorker(QObject):
    """Run pyserial reads, writes, line control and XModem off the UI thread."""

    data_received = pyqtSignal(bytes)
    opened = pyqtSignal()
    closed = pyqtSignal()
    error = pyqtSignal(str)
    status = pyqtSignal(str)
    statistics = pyqtSignal(int, int)
    line_states = pyqtSignal(bool, bool)
    transfer_progress = pyqtSignal(int, int, float)
    transfer_finished = pyqtSignal(bool, str)
    stopped = pyqtSignal()

    def __init__(self) -> None:
        super().__init__()
        self.port = SerialPort()
        self.poll_timer: QTimer | None = None
        self.loop_timer: QTimer | None = None
        self.rx_count = 0
        self.tx_count = 0
        self._loop_data = b""
        self._cancel_event = threading.Event()

    def _handle_io_error(self, message: str) -> None:
        """Close a failed connection so an unplugged device reports once."""
        if self.loop_timer is not None:
            self.loop_timer.stop()
        was_open = self.port.is_open
        self.port.close()
        if was_open:
            self.closed.emit()
        self.error.emit(message)
    def install_cancel_signal(self, signal: pyqtSignal) -> None:
        """Connect a UI-owned signal with a direct, thread-safe callback."""
        signal.connect(self._cancel_event.set, Qt.DirectConnection)

    @pyqtSlot()
    def start(self) -> None:
        self.poll_timer = QTimer(self)
        self.poll_timer.setInterval(10)
        self.poll_timer.timeout.connect(self._poll)
        self.poll_timer.start()
        self.loop_timer = QTimer(self)
        self.loop_timer.timeout.connect(self._send_loop_data)

    @pyqtSlot(dict)
    def open_port(self, values: dict) -> None:
        self._open_serial(values, preserve_statistics=False)

    @pyqtSlot(dict)
    def reopen_port(self, values: dict) -> None:
        self._open_serial(values, preserve_statistics=True)

    def _open_serial(self, values: dict, preserve_statistics: bool) -> None:
        try:
            self.port.open(PortSettings.from_dict(values))
            if not preserve_statistics:
                self.rx_count = self.tx_count = 0
                self.statistics.emit(0, 0)
            self.opened.emit()
            self.status.emit("Reconnected" if preserve_statistics else "Port opened")
            self.line_states.emit(*self.port.line_states())
        except Exception as exc:  # pyserial exposes several OS-specific errors
            self._handle_io_error(f"Open failed: {exc}")

    @pyqtSlot()
    def close_port(self) -> None:
        if self.loop_timer is not None:
            self.loop_timer.stop()
        self.port.close()
        self.closed.emit()
        self.status.emit("Port closed")

    @pyqtSlot()
    def stop(self) -> None:
        if self.poll_timer is not None:
            self.poll_timer.stop()
        if self.loop_timer is not None:
            self.loop_timer.stop()
        self.port.close()
        self.stopped.emit()
        # Quit from inside the owning thread after the serial object has been
        # closed.  This makes queued shutdown requests deterministic.
        from PyQt5.QtCore import QThread
        QThread.currentThread().quit()

    @pyqtSlot()
    def _poll(self) -> None:
        try:
            data = self.port.read_available()
            if data:
                self.rx_count += len(data)
                self.data_received.emit(data)
                self.statistics.emit(self.rx_count, self.tx_count)
        except Exception as exc:
            self._handle_io_error(f"Read failed: {exc}")

    @pyqtSlot(bytes)
    def send_data(self, data: bytes) -> None:
        try:
            written = self.port.write(data)
            self.tx_count += written
            self.statistics.emit(self.rx_count, self.tx_count)
        except Exception as exc:
            self._handle_io_error(f"Send failed: {exc}")

    @pyqtSlot(bytes, int)
    def configure_loop(self, data: bytes, interval_ms: int) -> None:
        self._loop_data = data
        if self.loop_timer is None:
            return
        if data and interval_ms > 0:
            self.loop_timer.setInterval(interval_ms)
            self.loop_timer.start()
        else:
            self.loop_timer.stop()

    @pyqtSlot()
    def _send_loop_data(self) -> None:
        if self._loop_data:
            self.send_data(self._loop_data)

    @pyqtSlot(bool)
    def set_dtr(self, value: bool) -> None:
        try:
            self.port.set_dtr(value)
            self.line_states.emit(*self.port.line_states())
        except Exception as exc:
            self.error.emit(f"DTR failed: {exc}")

    @pyqtSlot(bool)
    def set_rts(self, value: bool) -> None:
        try:
            self.port.set_rts(value)
            self.line_states.emit(*self.port.line_states())
        except Exception as exc:
            self.error.emit(f"RTS failed: {exc}")

    @pyqtSlot(str)
    def stm32_sequence(self, mode: str) -> None:
        """Apply common DTR/RTS wiring sequences used by STM32 adapters."""
        import time

        try:
            if mode == "reset":
                self.port.set_rts(False)
                self.port.set_dtr(False)  # assert NRST (active low)
                time.sleep(0.10)
                self.port.set_dtr(True)
            else:
                self.port.set_rts(True)  # BOOT0 high
                self.port.set_dtr(False)  # assert NRST
                time.sleep(0.10)
                self.port.set_dtr(True)
                time.sleep(0.10)
                self.port.set_rts(False)
            self.line_states.emit(*self.port.line_states())
            self.status.emit(f"STM32 {mode} sequence complete")
        except Exception as exc:
            self.error.emit(f"STM32 sequence failed: {exc}")

    @pyqtSlot(str)
    def send_file_xmodem(self, filename: str) -> None:
        self._cancel_event.clear()
        try:
            sender = XModemSender(
                self.port.write,
                self.port.read,
                self.transfer_progress.emit,
                self._cancel_event.is_set,
            )
            sender.send_file(filename)
            self.transfer_finished.emit(True, "XModem transfer complete")
        except Exception as exc:
            self.transfer_finished.emit(False, str(exc))
