"""Minimal, interoperable XModem sender using CRC-16/XMODEM."""

from __future__ import annotations

import time
from pathlib import Path
from typing import Callable


class XModemError(RuntimeError):
    """Raised when the receiver does not complete an XModem transfer."""


class XModemSender:
    """Blocking XModem sender intended to run in SerialWorker's thread.

    ``write`` and ``read`` are callbacks around the serial model.  The
    progress callback receives (bytes_sent, total_bytes, bytes_per_second).
    """

    SOH = 0x01
    STX = 0x02
    EOT = 0x04
    ACK = 0x06
    NAK = 0x15
    CAN = 0x18
    CRC_REQUEST = ord("C")

    def __init__(
        self,
        write: Callable[[bytes], int],
        read: Callable[[int], bytes],
        progress: Callable[[int, int, float], None] | None = None,
        cancelled: Callable[[], bool] | None = None,
    ) -> None:
        self.write = write
        self.read = read
        self.progress = progress or (lambda *_: None)
        self.cancelled = cancelled or (lambda: False)

    @staticmethod
    def crc16(data: bytes) -> int:
        crc = 0
        for byte in data:
            crc ^= byte << 8
            for _ in range(8):
                crc = ((crc << 1) ^ 0x1021) & 0xFFFF if crc & 0x8000 else (crc << 1) & 0xFFFF
        return crc

    def _wait_for(self, accepted: set[int], timeout: float) -> int | None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self.cancelled():
                return self.CAN
            chunk = self.read(1)
            if chunk:
                value = chunk[0]
                if value in accepted or value == self.CAN:
                    return value
        return None

    def send_file(self, filename: str) -> None:
        """Send a file with CRC XModem and retry each packet up to ten times."""
        path = Path(filename)
        total = path.stat().st_size
        with path.open("rb") as source:
            # Receivers commonly start with 'C' for CRC mode or NAK for
            # checksum mode.  CRC is preferred; checksum is also supported.
            start = self._wait_for({self.CRC_REQUEST, self.NAK}, 60.0)
            if start is None:
                raise XModemError("Timeout waiting for XModem receiver")
            if start == self.CAN:
                raise XModemError("Receiver cancelled the transfer")
            use_crc = start == self.CRC_REQUEST
            packet_size = 1024 if use_crc else 128
            block = 1
            sent = 0
            started = time.monotonic()

            while True:
                if self.cancelled():
                    self.write(bytes([self.CAN, self.CAN]))
                    raise XModemError("Transfer cancelled")
                payload = source.read(packet_size)
                if not payload:
                    break
                actual = len(payload)
                payload = payload.ljust(packet_size, b"\x1a")
                if use_crc:
                    packet = bytes([self.STX, block & 0xFF, 0xFF - (block & 0xFF)]) + payload
                    crc = self.crc16(payload)
                    packet += bytes([crc >> 8, crc & 0xFF])
                else:
                    checksum = sum(payload) & 0xFF
                    packet = bytes([self.SOH, block & 0xFF, 0xFF - (block & 0xFF)]) + payload + bytes([checksum])

                acknowledged = False
                for _ in range(10):
                    self.write(packet)
                    response = self._wait_for({self.ACK, self.NAK}, 10.0)
                    if response == self.ACK:
                        acknowledged = True
                        break
                    if response == self.CAN or response is None:
                        break
                if not acknowledged:
                    raise XModemError(f"No ACK for block {block}")
                sent += actual
                elapsed = max(time.monotonic() - started, 0.001)
                self.progress(sent, total, sent / elapsed)
                block = (block + 1) & 0xFF

            for _ in range(10):
                self.write(bytes([self.EOT]))
                response = self._wait_for({self.ACK, self.NAK}, 10.0)
                if response == self.ACK:
                    self.progress(total, total, total / max(time.monotonic() - started, 0.001))
                    return
                if response == self.CAN:
                    break
        raise XModemError("Receiver did not acknowledge EOT")
