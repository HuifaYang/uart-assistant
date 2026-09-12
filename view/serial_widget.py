"""The complete UI and controller facade for one serial-port tab."""

from __future__ import annotations

import datetime as dt
from pathlib import Path
import re

from PyQt5.QtCore import QEvent, QThread, QTimer, Qt, pyqtSignal, pyqtSlot
from PyQt5.QtGui import (
    QColor,
    QFont,
    QFontMetrics,
    QKeySequence,
    QPainter,
    QSyntaxHighlighter,
    QTextCharFormat,
    QTextCursor,
    QTextDocument,
)
from PyQt5.QtWidgets import (
    QCheckBox,
    QComboBox,
    QApplication,
    QFrame,
    QFileDialog,
    QGridLayout,
    QGroupBox,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QMessageBox,
    QPlainTextEdit,
    QProgressBar,
    QPushButton,
    QSizePolicy,
    QShortcut,
    QTabWidget,
    QSpinBox,
    QSplitter,
    QVBoxLayout,
    QWidget,
)

from controller.serial_worker import SerialWorker
from model.config import ConfigManager
from model.serial_port import DeviceInfo
from model.terminal_emulator import Cell, TerminalEmulator
from view.dialogs import MacroDialog


class KeywordHighlighter(QSyntaxHighlighter):
    """Highlight one literal keyword in the receive document."""

    def __init__(self, document) -> None:
        super().__init__(document)
        self.keyword = ""

    def set_keyword(self, keyword: str) -> None:
        self.keyword = keyword
        self.rehighlight()

    def highlightBlock(self, text: str) -> None:  # noqa: N802 - Qt API name
        if not self.keyword:
            return
        format_ = QTextCharFormat()
        format_.setForeground(QColor("#ffd166"))
        format_.setBackground(QColor("#493c1b"))
        start = 0
        while True:
            index = text.find(self.keyword, start)
            if index < 0:
                return
            self.setFormat(index, len(self.keyword), format_)
            start = index + len(self.keyword)


class TerminalView(QWidget):
    """Flicker-free fixed-grid terminal view backed by the emulator.

    The previous implementation regenerated a complete HTML document for
    every received packet and every locally echoed key.  That is expensive
    and visibly flashes after a large boot log.  This widget paints only the
    current terminal grid with QPainter, so input and high-rate output do not
    rebuild a QTextDocument.
    """

    input_requested = pyqtSignal(bytes)

    _special_keys = {
        Qt.Key_Return: b"\r",
        Qt.Key_Enter: b"\r",
        Qt.Key_Backspace: b"\x7f",
        Qt.Key_Tab: b"\t",
        Qt.Key_Up: b"\x1b[A",
        Qt.Key_Down: b"\x1b[B",
        Qt.Key_Right: b"\x1b[C",
        Qt.Key_Left: b"\x1b[D",
        Qt.Key_Home: b"\x1b[H",
        Qt.Key_End: b"\x1b[F",
        Qt.Key_Delete: b"\x1b[3~",
        Qt.Key_Insert: b"\x1b[2~",
        Qt.Key_PageUp: b"\x1b[5~",
        Qt.Key_PageDown: b"\x1b[6~",
    }

    def __init__(self, parent=None) -> None:
        super().__init__(parent)
        # Tab belongs to the remote shell.  StrongFocus plus the
        # focusNextPrevChild override below prevents Qt from moving focus to
        # the next button or input field in the main window.
        self.setFocusPolicy(Qt.StrongFocus)
        terminal_font = QFont("DejaVu Sans Mono", 11)
        terminal_font.setStyleHint(QFont.Monospace)
        terminal_font.setFixedPitch(True)
        terminal_font.setKerning(False)
        self.setFont(terminal_font)
        self.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Expanding)
        self.setAttribute(Qt.WA_OpaquePaintEvent, True)
        self.setAttribute(Qt.WA_InputMethodEnabled, True)
        self.emulator = TerminalEmulator()
        self.local_echo = False
        self.search_query = ""
        self.search_case_sensitive = False
        self.search_whole_word = False
        self.search_matches: list[tuple[int, int, int]] = []
        self.search_index = -1
        self._last_columns = 0
        self._last_rows = 0
        self._scroll_offset = 0
        self._line_height = 1
        self._char_width = 1

    def _send(self, data: bytes) -> None:
        """Send bytes to the remote terminal and follow the live screen."""
        self._scroll_offset = 0
        self.input_requested.emit(data)

    def keyPressEvent(self, event) -> None:  # noqa: N802 - Qt API name
        modifiers = event.modifiers()
        if modifiers & Qt.ControlModifier and event.key() not in (Qt.Key_Shift, Qt.Key_Alt):
            key = event.key()
            if Qt.Key_A <= key <= Qt.Key_Z:
                self._send(bytes([key - Qt.Key_A + 1]))
                return
            if key == Qt.Key_Space:
                self._send(b"\x00")
                return
            if key == Qt.Key_BracketLeft:
                self._send(b"\x1b")
                return
        if event.key() in (Qt.Key_Tab, Qt.Key_Backtab):
            # Qt represents Shift+Tab as Key_Backtab on some platforms.
            # Both forms must be sent to the remote shell instead of being
            # interpreted as local focus navigation.
            self._send(
                b"\x1b[Z"
                if event.key() == Qt.Key_Backtab or modifiers & Qt.ShiftModifier
                else b"\t"
            )
            return
        if event.key() in self._special_keys:
            self._send(self._special_keys[event.key()])
            return
        if modifiers & Qt.ControlModifier and event.key() == Qt.Key_V:
            text = QApplication.clipboard().text()
            if text:
                self._send(text.replace("\n", "\r").encode("utf-8"))
            return
        text = event.text()
        if text and not (modifiers & (Qt.ControlModifier | Qt.AltModifier)):
            data = text.encode("utf-8")
            self._send(data)
            if self.local_echo:
                self.emulator.feed(data)
                self.update()
            return
        super().keyPressEvent(event)

    def event(self, event):  # noqa: N802 - Qt API name
        """Intercept Tab before QTextEdit turns it into focus navigation."""
        if event.type() == QEvent.KeyPress and event.key() in (Qt.Key_Tab, Qt.Key_Backtab):
            self._send(
                b"\x1b[Z"
                if event.key() == Qt.Key_Backtab or event.modifiers() & Qt.ShiftModifier
                else b"\t"
            )
            return True
        return super().event(event)

    def focusNextPrevChild(self, next_: bool) -> bool:  # noqa: N802 - Qt API name
        """Never let Tab leave the remote terminal widget."""
        # This callback can be entered by Qt while it is already resolving a
        # focus transition.  Calling setFocus() here re-enters that process
        # and can make Shift+Tab hang.  Tab is handled by event() above, so
        # returning True here only consumes any fallback focus request.
        return True

    def write_bytes(self, data: bytes, render: bool = True) -> None:
        """Feed the emulator and optionally redraw the fixed terminal grid.

        The state must continue advancing while the Monitor tab is visible,
        but rebuilding hidden terminal HTML is wasted UI work during a boot
        log burst.
        """
        self.emulator.feed(data)
        self._invalidate_search()
        if render:
            self.update()

    def refresh(self) -> None:
        """Redraw the current emulator state after the view becomes visible."""
        self.update()

    @property
    def cr_as_newline(self) -> bool:
        return self.emulator.cr_as_newline

    @cr_as_newline.setter
    def cr_as_newline(self, value: bool) -> None:
        self.emulator.cr_as_newline = value

    def resizeEvent(self, event) -> None:  # noqa: N802 - Qt API name
        super().resizeEvent(event)
        metrics = QFontMetrics(self.font())
        # Use a real glyph advance from the active fixed-pitch font.  Using
        # the UI font or a proportional sample here shifts the block cursor.
        self._char_width = max(1, metrics.horizontalAdvance("0"))
        self._line_height = max(1, metrics.lineSpacing())
        columns = max(20, self.width() // self._char_width)
        rows = max(5, self.height() // self._line_height)
        if (columns, rows) != (self._last_columns, self._last_rows):
            self._last_columns, self._last_rows = columns, rows
            self.emulator.resize(columns, rows)
            self._invalidate_search()
            self.update()
            parent = self.parentWidget()
            if parent is not None and hasattr(parent, "_update_terminal_size_label"):
                parent._update_terminal_size_label()

    def clear(self) -> None:
        """Clear the terminal screen and its scrollback."""
        self.emulator.reset()
        self._scroll_offset = 0
        if self.isVisible():
            self._invalidate_search()
        else:
            self.search_matches.clear()
            self.search_index = -1
        self.update()

    def _render(self) -> None:
        """Compatibility alias for callers from the earlier text view."""
        self.update()

    def wheelEvent(self, event) -> None:  # noqa: N802 - Qt API name
        """Scroll through terminal history without adding a Qt scrollbar."""
        steps = event.angleDelta().y() // 120
        max_offset = max(0, len(self.emulator.visible_rows()) - self.emulator.rows)
        # Qt reports a positive angleDelta().y() for wheel-up.  A larger
        # offset means an earlier slice of the terminal scrollback, so wheel
        # up must increase the offset and wheel down must decrease it.
        self._scroll_offset = max(0, min(max_offset, self._scroll_offset + steps * 3))
        self.update()
        event.accept()

    def paintEvent(self, event) -> None:  # noqa: N802 - Qt API name
        """Paint the visible rows directly, avoiding document replacement."""
        painter = QPainter(self)
        painter.fillRect(self.rect(), QColor("#11161b"))
        metrics = QFontMetrics(self.font())
        line_height = self._line_height or max(1, metrics.lineSpacing())
        char_width = self._char_width or max(1, metrics.horizontalAdvance("0"))
        visible_count = max(1, self.height() // line_height)
        all_rows = self.emulator.visible_rows()
        end = max(0, len(all_rows) - self._scroll_offset)
        start = max(0, end - visible_count)
        # Convert the emulator's screen-local cursor to the same global row
        # space used by the visible scrollback slice.
        screen_start = len(all_rows) - len(self.emulator.buffer)
        cursor_global = screen_start + max(0, min(self.emulator.cursor_y, self.emulator.rows - 1))
        cursor_column = min(self.emulator.cursor_x, self.emulator.columns - 1)

        for screen_row, row_index in enumerate(range(start, end)):
            row = all_rows[row_index]
            y = screen_row * line_height
            column = 0
            while column < len(row):
                cell = row[column]
                is_cursor = (
                    self.emulator.cursor_visible
                    and self._scroll_offset == 0
                    and row_index == cursor_global
                    and column == cursor_column
                )
                foreground, background, bold = self._cell_style(cell, is_cursor)
                run_start = column
                column += 1
                while column < len(row):
                    next_cell = row[column]
                    next_cursor = (
                        self.emulator.cursor_visible
                        and self._scroll_offset == 0
                        and row_index == cursor_global
                        and column == cursor_column
                    )
                    if self._cell_style(next_cell, next_cursor) != (foreground, background, bold):
                        break
                    column += 1
                run = "".join(item.char for item in row[run_start:column])
                x = run_start * char_width
                painter.fillRect(x, y, (column - run_start) * char_width, line_height, QColor(background))
                font = self.font()
                font.setBold(bold)
                painter.setFont(font)
                painter.setPen(QColor(foreground))
                painter.drawText(x, y + metrics.ascent(), run)

        if self.search_matches:
            for match_index, (row_index, start_column, end_column) in enumerate(self.search_matches):
                if not (start <= row_index < end):
                    continue
                screen_row = row_index - start
                painter.fillRect(
                    start_column * char_width,
                    screen_row * line_height,
                    (end_column - start_column) * char_width,
                    line_height,
                    QColor(255, 209, 102, 110 if match_index == self.search_index else 55),
                )
        painter.end()

    def _invalidate_search(self) -> None:
        if not self.isVisible():
            self.search_matches.clear()
            self.search_index = -1
        elif self.search_query:
            self._search_terminal()
        else:
            self.search_matches.clear()
            self.search_index = -1

    def _search_terminal(self) -> None:
        query = self.search_query
        self.search_matches.clear()
        self.search_index = -1
        if not query:
            self.update()
            return
        if self.search_whole_word:
            escaped = r"\b".join(re.escape(part) for part in query.split())
            pattern = rf"(?<!\w){escaped}(?!\w)"
            flags = 0 if self.search_case_sensitive else re.IGNORECASE
            matcher = re.compile(pattern, flags)

            def find_match(line: str, start: int = 0):
                return matcher.search(line, start)
        else:
            lower_query = query.lower()

            def find_match(line: str, start: int = 0):
                if self.search_case_sensitive:
                    position = line.find(query, start)
                else:
                    position = line.lower().find(lower_query, start)
                return None if position < 0 else (position, position + len(query))

        for row_index, row in enumerate(self.emulator.visible_rows()):
            line = "".join(cell.char for cell in row)
            start = 0
            while True:
                result = find_match(line, start)
                if not result:
                    break
                match_start, match_end = result.span() if self.search_whole_word else result
                self.search_matches.append((row_index, match_start, match_end))
                start = max(match_end, match_start + 1)
        self.update()

    def _find_terminal_next(self) -> None:
        if not self.search_matches:
            return
        self.search_index = (self.search_index + 1) % len(self.search_matches)
        self._scroll_to_terminal_match()

    def _find_terminal_previous(self) -> None:
        if not self.search_matches:
            return
        self.search_index = (self.search_index - 1) % len(self.search_matches)
        self._scroll_to_terminal_match()

    def _scroll_to_terminal_match(self) -> None:
        row_index = self.search_matches[self.search_index][0]
        visible_rows = self.emulator.visible_rows()
        max_offset = max(0, len(visible_rows) - self.emulator.rows)
        preferred_offset = len(visible_rows) - row_index - self.emulator.rows
        self._scroll_offset = max(0, min(max_offset, preferred_offset))
        self.update()

    @staticmethod
    def _cell_style(cell: Cell, cursor: bool) -> tuple[str, str, bool]:
        foreground, background = cell.foreground, cell.background
        if cell.reverse or cursor:
            foreground, background = background, foreground
        return foreground, background, cell.bold



class SerialWidget(QWidget):
    """One independent serial session, including its worker thread."""

    cancel_transfer = pyqtSignal()
    request_scan = pyqtSignal()
    open_requested = pyqtSignal(dict)
    close_requested = pyqtSignal()
    send_requested = pyqtSignal(bytes)
    loop_requested = pyqtSignal(bytes, int)
    dtr_requested = pyqtSignal(bool)
    rts_requested = pyqtSignal(bool)
    stm32_requested = pyqtSignal(str)
    xmodem_requested = pyqtSignal(str)
    stop_requested = pyqtSignal()
    reconnect_requested = pyqtSignal(dict)
    status_changed = pyqtSignal(str)
    statistics_changed = pyqtSignal(int, int)

    def __init__(self, config: ConfigManager, parent=None) -> None:
        super().__init__(parent)
        self.config = config
        self.worker = SerialWorker()
        self.thread = QThread(self)
        self.worker.moveToThread(self.thread)
        self.open_requested.connect(self.worker.open_port)
        self.close_requested.connect(self.worker.close_port)
        self.reconnect_requested.connect(self.worker.reopen_port)
        self.send_requested.connect(self.worker.send_data)
        self.loop_requested.connect(self.worker.configure_loop)
        self.dtr_requested.connect(self.worker.set_dtr)
        self.rts_requested.connect(self.worker.set_rts)
        self.stm32_requested.connect(self.worker.stm32_sequence)
        self.xmodem_requested.connect(self.worker.send_file_xmodem)
        self.stop_requested.connect(self.worker.stop)
        self.thread.started.connect(self.worker.start)
        self.worker.install_cancel_signal(self.cancel_transfer)
        self.worker.data_received.connect(self._on_data)
        self.worker.opened.connect(self._on_opened)
        self.worker.closed.connect(self._on_closed)
        self.worker.error.connect(self._on_error)
        self.worker.statistics.connect(self._update_statistics)
        self.worker.line_states.connect(self._update_line_states)
        self.worker.status.connect(self._set_status)
        self.worker.transfer_progress.connect(self._transfer_progress)
        self.worker.transfer_finished.connect(self._transfer_finished)
        self._devices: list[DeviceInfo] = []
        self._rx_offset = 0
        self._history: list[str] = []
        self._port_open = False
        self._manual_close = False
        self._reconnect_active = False
        self._reconnect_pending = False
        self._last_port_settings: dict | None = None
        self._rx_count = 0
        self._tx_count = 0
        self._status_text = "未连接"
        # Serial data can arrive much faster than a QTextEdit can repaint.
        # Keep reception lightweight and let a UI timer consume data in
        # bounded batches.  The worker still counts every received byte.
        self._pending_rx = bytearray()
        self._pending_rx_limit = 4 * 1024 * 1024
        self._rx_flush_limit = 32 * 1024
        self._display_dropped_bytes = 0
        self._rx_flush_timer = QTimer(self)
        self._rx_flush_timer.setInterval(33)
        self._rx_flush_timer.timeout.connect(self._flush_received_data)
        self._reconnect_timer = QTimer(self)
        self._reconnect_timer.setInterval(1500)
        self._reconnect_timer.timeout.connect(self._try_reconnect)
        self._macros = config.load_macros()
        self._build_ui()
        self._rx_flush_timer.start()
        self.thread.start()

    @property
    def connected(self) -> bool:
        return self._port_open

    @property
    def is_open(self) -> bool:
        return self._port_open

    @property
    def statistics(self) -> tuple[int, int]:
        return self._rx_count, self._tx_count

    def status_text(self) -> str:
        return self._status_text

    def connection_summary(self) -> str:
        port = self.port_combo.currentData() or self.port_combo.currentText().strip()
        if not port or port == "未检测到可用串口设备":
            return ""
        try:
            baudrate = int(self.baud_combo.currentText())
        except ValueError:
            return str(port)
        return f"{port} · {baudrate}"

    def _build_ui(self) -> None:
        ui_font = QFont("Noto Sans CJK SC")
        ui_font.setPointSize(10)
        self.setFont(ui_font)

        self.port_combo = QComboBox()
        self.port_combo.setEditable(True)
        self.port_combo.setMaxVisibleItems(8)
        self.port_combo.view().setMaximumHeight(8 * 30)
        self.port_combo.setPlaceholderText("选择或输入设备路径，例如 /dev/ttyS2")
        self.port_combo.setMinimumWidth(240)
        self.port_combo.setToolTip("设备：选择或输入串口路径")
        self.port_combo.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Fixed)
        self.refresh_button = QPushButton("刷新")
        self.refresh_button.clicked.connect(self.request_scan)
        self.baud_combo = QComboBox()
        self.baud_combo.setEditable(True)
        self.baud_combo.addItems(["9600", "19200", "38400", "57600", "115200", "230400", "460800", "921600", "1000000", "2000000"])
        self.baud_combo.setCurrentText("115200")
        self.baud_combo.setToolTip("波特率")
        self.baud_combo.setMinimumWidth(104)
        self.bits_combo = QComboBox()
        self.bits_combo.addItems(["5", "6", "7", "8"])
        self.bits_combo.setCurrentText("8")
        self.bits_combo.setToolTip("数据位")
        self.bits_combo.setMinimumWidth(66)
        self.stop_combo = QComboBox()
        self.stop_combo.addItems(["1", "1.5", "2"])
        self.stop_combo.setToolTip("停止位")
        self.stop_combo.setMinimumWidth(66)
        self.parity_combo = QComboBox()
        self.parity_combo.addItem("无", "N")
        self.parity_combo.addItem("奇校验", "O")
        self.parity_combo.addItem("偶校验", "E")
        self.parity_combo.setToolTip("校验位")
        self.parity_combo.setMinimumWidth(82)
        self.flow_combo = QComboBox()
        self.flow_combo.addItem("无", "none")
        self.flow_combo.addItem("硬件 (RTS/CTS)", "hardware")
        self.flow_combo.addItem("软件 (XON/XOFF)", "software")
        self.flow_combo.setToolTip("流控")
        self.flow_combo.setMinimumWidth(118)
        self.open_button = QPushButton("打开")
        self.open_button.setObjectName("primaryButton")
        self.open_button.clicked.connect(self._toggle_open)
        self.auto_reconnect_check = QCheckBox("断线自动重连")
        self.auto_reconnect_check.setChecked(bool(self.config.load_value("serial/auto_reconnect", False, type_=bool)))
        self.auto_reconnect_check.toggled.connect(self._set_auto_reconnect)

        self.connection_panel = QWidget()
        self.connection_panel.setObjectName("toolbar")
        self.connection_panel.setSizePolicy(QSizePolicy.Preferred, QSizePolicy.Maximum)
        connection_layout = QHBoxLayout(self.connection_panel)
        connection_layout.setContentsMargins(10, 6, 10, 6)
        connection_layout.setSpacing(6)
        device_label = QLabel("设备")
        device_label.setObjectName("fieldLabel")
        baud_label = QLabel("波特率")
        baud_label.setObjectName("fieldLabel")
        bits_label = QLabel("数据位")
        bits_label.setObjectName("fieldLabel")
        stop_label = QLabel("停止位")
        stop_label.setObjectName("fieldLabel")
        parity_label = QLabel("校验")
        parity_label.setObjectName("fieldLabel")
        flow_label = QLabel("流控")
        flow_label.setObjectName("fieldLabel")
        connection_layout.addWidget(device_label)
        connection_layout.addWidget(self.port_combo, 1)
        connection_layout.addWidget(self.refresh_button)
        connection_layout.addWidget(baud_label)
        connection_layout.addWidget(self.baud_combo)
        connection_layout.addWidget(bits_label)
        connection_layout.addWidget(self.bits_combo)
        connection_layout.addWidget(stop_label)
        connection_layout.addWidget(self.stop_combo)
        connection_layout.addWidget(parity_label)
        connection_layout.addWidget(self.parity_combo)
        connection_layout.addWidget(flow_label)
        connection_layout.addWidget(self.flow_combo)
        connection_layout.addWidget(self.auto_reconnect_check)
        connection_layout.addWidget(self.open_button)

        self.receive = QPlainTextEdit()
        receive_font = QFont("DejaVu Sans Mono")
        receive_font.setStyleHint(QFont.Monospace)
        receive_font.setPointSize(10)
        self.receive.setFont(receive_font)
        self.receive.setReadOnly(True)
        self.receive.setLineWrapMode(QPlainTextEdit.NoWrap)
        self.receive.setUndoRedoEnabled(False)
        self.receive.setFrameShape(QFrame.NoFrame)
        # Keep the monitor useful during long boot logs without allowing the
        # document to grow indefinitely and make every append slower.
        max_log_lines = max(10000, min(30000, int(self.config.load_value("monitor/max_log_lines", 10000, type_=int) or 10000)))
        self.receive.setMaximumBlockCount(max_log_lines)
        self.receive.setPlaceholderText("接收到的数据将显示在这里")
        self.receive.setObjectName("monitorView")
        self.highlighter = KeywordHighlighter(self.receive.document())
        self.timestamp_check = QCheckBox("时间戳 (ms)")
        self.timestamp_check.setChecked(bool(self.config.load_value("monitor/timestamp", True, type_=bool)))
        self.timestamp_check.toggled.connect(
            lambda checked: self.config.save_value("monitor/timestamp", checked)
        )
        self.autoscroll_check = QCheckBox("自动滚屏")
        self.autoscroll_check.setChecked(True)
        self.hex_receive = QCheckBox("HEX")
        self.hex_receive.setVisible(False)
        self.hex_receive.toggled.connect(self._reset_display)
        self.display_mode_combo = QComboBox()
        self.display_mode_combo.addItems(["普通", "HEX", "ASCII + HEX"])
        self.display_mode_combo.setCurrentText(
            str(self.config.load_value("monitor/display_mode", "普通"))
        )
        if self.display_mode_combo.currentIndex() < 0:
            self.display_mode_combo.setCurrentIndex(0)
        self.display_mode_combo.currentIndexChanged.connect(self._set_display_mode)
        self.keyword_edit = QLineEdit()
        self.keyword_edit.setPlaceholderText("关键词高亮")
        self.keyword_edit.textChanged.connect(self.highlighter.set_keyword)
        self.pause_button = QPushButton("暂停")
        self.pause_button.setCheckable(True)
        self.clear_button = QPushButton("清空")
        self.clear_button.clicked.connect(self._clear_receive)
        self.save_log_button = QPushButton("保存日志")
        self.save_log_button.clicked.connect(self._save_log)
        self.log_limit_spin = QSpinBox()
        self.log_limit_spin.setRange(10000, 30000)
        self.log_limit_spin.setSingleStep(1000)
        self.log_limit_spin.setValue(max_log_lines)
        self.log_limit_spin.setSuffix(" 行")
        self.log_limit_spin.setFixedWidth(94)
        self.log_limit_spin.valueChanged.connect(
            lambda value: self.config.save_value("monitor/max_log_lines", value)
        )
        receive_tools = QHBoxLayout()
        receive_tools.setContentsMargins(0, 0, 0, 0)
        receive_tools.setSpacing(6)
        receive_tools.addWidget(self.timestamp_check)
        receive_tools.addWidget(self.autoscroll_check)
        display_mode_label = QLabel("显示")
        display_mode_label.setObjectName("fieldLabel")
        receive_tools.addWidget(display_mode_label)
        receive_tools.addWidget(self.display_mode_combo)
        receive_tools.addWidget(self.log_limit_spin)
        receive_tools.addWidget(self.keyword_edit, 1)
        receive_tools.addWidget(self.pause_button)
        receive_tools.addWidget(self.clear_button)
        receive_tools.addWidget(self.save_log_button)
        receive_panel = QWidget()
        receive_panel.setObjectName("contentPanel")
        receive_layout = QVBoxLayout(receive_panel)
        receive_layout.setContentsMargins(0, 0, 0, 0)
        receive_layout.setSpacing(5)
        receive_layout.addLayout(receive_tools)
        self.search_panel = QWidget()
        self.search_panel.setObjectName("toolbar")
        self.search_panel.setVisible(False)
        search_layout = QHBoxLayout(self.search_panel)
        search_layout.setContentsMargins(10, 5, 10, 5)
        search_layout.setSpacing(6)
        search_label = QLabel("搜索")
        search_label.setObjectName("fieldLabel")
        self.search_edit = QLineEdit()
        self.search_edit.setClearButtonEnabled(True)
        self.search_edit.setPlaceholderText("输入搜索内容")
        self.search_edit.textChanged.connect(lambda: self.search_status_label.setText(""))
        self.search_case_check = QCheckBox("区分大小写")
        self.search_case_check.setChecked(bool(self.config.load_value("monitor/search_case_sensitive", False, type_=bool)))
        self.search_case_check.toggled.connect(
            lambda checked: self.config.save_value("monitor/search_case_sensitive", checked)
        )
        self.search_word_check = QCheckBox("全词匹配")
        self.search_word_check.setChecked(bool(self.config.load_value("monitor/search_whole_word", False, type_=bool)))
        self.search_word_check.toggled.connect(
            lambda checked: self.config.save_value("monitor/search_whole_word", checked)
        )
        self.search_prev_button = QPushButton("上一条")
        self.search_next_button = QPushButton("下一条")
        self.search_close_button = QPushButton("关闭")
        self.search_status_label = QLabel("")
        self.search_status_label.setObjectName("mutedLabel")
        self.search_status_label.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Fixed)
        search_layout.addWidget(search_label)
        search_layout.addWidget(self.search_edit, 1)
        search_layout.addWidget(self.search_case_check)
        search_layout.addWidget(self.search_word_check)
        search_layout.addWidget(self.search_prev_button)
        search_layout.addWidget(self.search_next_button)
        search_layout.addWidget(self.search_status_label)
        search_layout.addWidget(self.search_close_button)
        self.search_edit.returnPressed.connect(self._find_next)
        self.search_prev_button.clicked.connect(self._find_previous)
        self.search_next_button.clicked.connect(self._find_next)
        self.search_close_button.clicked.connect(self._hide_search)
        receive_layout.addWidget(self.search_panel)

        self.terminal_search_panel = QWidget()
        self.terminal_search_panel.setObjectName("toolbar")
        self.terminal_search_panel.setVisible(False)
        terminal_search_layout = QHBoxLayout(self.terminal_search_panel)
        terminal_search_layout.setContentsMargins(10, 5, 10, 5)
        terminal_search_layout.setSpacing(6)
        terminal_search_label = QLabel("搜索")
        terminal_search_label.setObjectName("fieldLabel")
        self.terminal_search_edit = QLineEdit()
        self.terminal_search_edit.setClearButtonEnabled(True)
        self.terminal_search_edit.setPlaceholderText("输入终端搜索内容")
        self.terminal_search_edit.textChanged.connect(self._set_terminal_search_query)
        self.terminal_search_case_check = QCheckBox("区分大小写")
        self.terminal_search_word_check = QCheckBox("全词匹配")
        self.terminal_search_case_check.toggled.connect(self._set_terminal_search_options)
        self.terminal_search_word_check.toggled.connect(self._set_terminal_search_options)
        self.terminal_search_prev_button = QPushButton("上一条")
        self.terminal_search_next_button = QPushButton("下一条")
        self.terminal_search_close_button = QPushButton("关闭")
        self.terminal_search_status_label = QLabel("")
        self.terminal_search_status_label.setObjectName("mutedLabel")
        self.terminal_search_status_label.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Fixed)
        terminal_search_layout.addWidget(terminal_search_label)
        terminal_search_layout.addWidget(self.terminal_search_edit, 1)
        terminal_search_layout.addWidget(self.terminal_search_case_check)
        terminal_search_layout.addWidget(self.terminal_search_word_check)
        terminal_search_layout.addWidget(self.terminal_search_prev_button)
        terminal_search_layout.addWidget(self.terminal_search_next_button)
        terminal_search_layout.addWidget(self.terminal_search_status_label)
        terminal_search_layout.addWidget(self.terminal_search_close_button)
        self.terminal_search_edit.returnPressed.connect(self._terminal_search_next)
        self.terminal_search_prev_button.clicked.connect(self._terminal_search_previous)
        self.terminal_search_next_button.clicked.connect(self._terminal_search_next)
        self.terminal_search_close_button.clicked.connect(self._hide_search)
        receive_layout.addWidget(self.receive)

        QShortcut(QKeySequence("Ctrl+F"), self, self._show_search, context=Qt.WidgetWithChildrenShortcut)
        QShortcut(QKeySequence("Ctrl+L"), self, self._clear_receive, context=Qt.WidgetWithChildrenShortcut)
        QShortcut(QKeySequence("Ctrl+S"), self, self._save_log, context=Qt.WidgetWithChildrenShortcut)
        QShortcut(QKeySequence("Ctrl+Return"), self, self._send_current, context=Qt.WidgetWithChildrenShortcut)
        QShortcut(QKeySequence("Escape"), self, self._hide_search, context=Qt.WidgetWithChildrenShortcut)

        self.terminal = TerminalView()
        self.terminal.input_requested.connect(self._send_terminal_input)
        terminal_tools = QHBoxLayout()
        self.terminal_echo = QCheckBox("本地回显")
        self.terminal_echo.setChecked(False)
        self.terminal_echo.toggled.connect(self._set_terminal_echo)
        self.terminal_size_label = QLabel("-- x --")
        self.terminal_size_label.setObjectName("mutedLabel")
        self.terminal_cr_newline = QCheckBox("CR 转换为换行")
        # Keep standard terminal semantics by default.  A bare CR is used by
        # shells to redraw a line; treating it as LF breaks history editing.
        self.terminal_cr_newline.setChecked(False)
        self.terminal_cr_newline.toggled.connect(self._set_terminal_cr_mode)
        terminal_clear = QPushButton("清空终端")
        terminal_clear.clicked.connect(self.terminal.clear)
        terminal_hint = QLabel("Linux 控制台：键盘输入将直接发送到串口")
        terminal_hint.setObjectName("mutedLabel")
        terminal_tools.addWidget(terminal_hint)
        terminal_tools.addStretch()
        terminal_tools.addWidget(self.terminal_size_label)
        terminal_tools.addWidget(self.terminal_echo)
        terminal_tools.addWidget(self.terminal_cr_newline)
        terminal_tools.addWidget(terminal_clear)
        terminal_panel = QWidget()
        terminal_panel.setObjectName("contentPanel")
        terminal_layout = QVBoxLayout(terminal_panel)
        terminal_layout.setContentsMargins(0, 0, 0, 0)
        terminal_layout.setSpacing(5)
        terminal_layout.addWidget(self.terminal_search_panel)
        terminal_layout.addLayout(terminal_tools)
        terminal_layout.addWidget(self.terminal)
        self.output_tabs = QTabWidget()
        self.output_tabs.setObjectName("outputTabs")
        self.output_tabs.setDocumentMode(True)
        self.output_tabs.addTab(receive_panel, "监视器")
        self.output_tabs.addTab(terminal_panel, "终端")
        self.output_tabs.currentChanged.connect(self._on_output_tab_changed)

        self.send_edit = QLineEdit()
        self.send_edit.setClearButtonEnabled(True)
        self.send_edit.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Fixed)
        self.send_edit.setPlaceholderText("输入 ASCII 文本或十六进制字节，例如：01 03 00 00")
        self.send_mode = QComboBox()
        self.send_mode.addItems(["ASCII", "HEX"])
        self.send_mode.setFixedWidth(84)
        self.suffix_combo = QComboBox()
        self.suffix_combo.addItem("无后缀", b"")
        self.suffix_combo.addItem("CRLF (\\r\\n)", b"\r\n")
        self.suffix_combo.addItem("LF (\\n)", b"\n")
        self.suffix_combo.addItem("CR (\\r)", b"\r")
        self.suffix_combo.setFixedWidth(104)
        self.history_combo = QComboBox()
        self.history_combo.setEditable(True)
        self.history_combo.setPlaceholderText("发送历史")
        self.history_combo.activated.connect(lambda index: self.send_edit.setText(self.history_combo.itemText(index)))
        self.history_combo.setMinimumWidth(120)
        self.send_button = QPushButton("发送")
        self.send_button.setObjectName("primaryButton")
        self.send_button.clicked.connect(self._send_current)
        self.send_button.setFixedWidth(78)
        self.loop_check = QCheckBox("循环发送")
        self.loop_check.toggled.connect(self._update_loop)
        self.loop_interval = QSpinBox()
        self.loop_interval.setRange(10, 3600000)
        self.loop_interval.setValue(1000)
        self.loop_interval.setSuffix(" 毫秒")
        self.loop_interval.setFixedWidth(96)
        self.send_panel = QWidget()
        self.send_panel.setObjectName("toolbar")
        self.send_panel.setSizePolicy(QSizePolicy.Preferred, QSizePolicy.Maximum)
        send_layout = QVBoxLayout(self.send_panel)
        send_layout.setContentsMargins(10, 6, 10, 6)
        send_layout.setSpacing(6)
        send_options = QHBoxLayout()
        send_options.setSpacing(6)
        send_options.addWidget(self.send_mode)
        send_options.addWidget(self.suffix_combo)
        send_options.addWidget(self.send_edit, 1)
        send_options.addWidget(self.send_button)
        send_layout.addLayout(send_options)
        send_controls = QHBoxLayout()
        send_controls.setSpacing(6)
        send_controls.addWidget(self.loop_check)
        send_controls.addWidget(self.loop_interval)
        self.history_combo.setMinimumWidth(100)
        send_controls.addWidget(self.history_combo, 1)
        send_layout.addLayout(send_controls)
        self.dtr_button = QPushButton("DTR 低电平")
        self.dtr_button.setCheckable(True)
        self.dtr_button.toggled.connect(self.dtr_requested.emit)
        self.rts_button = QPushButton("RTS 低电平")
        self.rts_button.setCheckable(True)
        self.rts_button.toggled.connect(self.rts_requested.emit)
        reset_button = QPushButton("STM32 复位")
        reset_button.clicked.connect(lambda: self.stm32_requested.emit("reset"))
        boot_button = QPushButton("进入 Bootloader")
        boot_button.clicked.connect(lambda: self.stm32_requested.emit("bootloader"))
        self.xmodem_button = QPushButton("发送 XModem 文件")
        self.xmodem_button.clicked.connect(self._choose_xmodem_file)
        self.cancel_xmodem_button = QPushButton("取消传输")
        self.cancel_xmodem_button.setEnabled(False)
        self.cancel_xmodem_button.clicked.connect(self.cancel_transfer.emit)
        self.transfer_progress = QProgressBar()
        self.transfer_progress.setRange(0, 100)
        self.transfer_progress.setFixedHeight(8)
        self.transfer_progress.setMinimumWidth(180)
        self.transfer_progress.setTextVisible(False)
        self.transfer_progress.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Fixed)
        self.transfer_label = QLabel("XModem 就绪")
        self.transfer_label.setObjectName("mutedLabel")
        self.transfer_label.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Fixed)
        self.transfer_label.setAlignment(Qt.AlignVCenter | Qt.AlignLeft)
        macro_button = QPushButton("编辑宏命令")
        macro_button.clicked.connect(self._edit_macros)
        self.macro_layout = QHBoxLayout()
        self._rebuild_macros()
        self.controls_panel = QWidget()
        self.controls_panel.setObjectName("toolbar")
        self.controls_panel.setSizePolicy(QSizePolicy.Preferred, QSizePolicy.Maximum)
        controls_grid = QGridLayout(self.controls_panel)
        controls_grid.setContentsMargins(10, 6, 10, 6)
        controls_grid.setHorizontalSpacing(6)
        controls_grid.setVerticalSpacing(6)
        serial_label = QLabel("串口控制")
        serial_label.setObjectName("sectionLabel")
        stm32_label = QLabel("STM32")
        stm32_label.setObjectName("sectionLabel")
        xmodem_label = QLabel("XModem")
        xmodem_label.setObjectName("sectionLabel")
        macro_label = QLabel("宏命令")
        macro_label.setObjectName("sectionLabel")
        control_height = 30
        for widget in (
            self.refresh_button,
            self.baud_combo,
            self.bits_combo,
            self.stop_combo,
            self.parity_combo,
            self.flow_combo,
            self.open_button,
            self.auto_reconnect_check,
            self.send_mode,
            self.suffix_combo,
            self.send_button,
            self.loop_interval,
            self.history_combo,
            self.send_edit,
            self.display_mode_combo,
            self.log_limit_spin,
            self.dtr_button,
            self.rts_button,
            self.xmodem_button,
            self.cancel_xmodem_button,
            macro_button,
            reset_button,
            boot_button,
            self.search_edit,
            self.search_case_check,
            self.search_word_check,
            self.search_prev_button,
            self.search_next_button,
            self.search_close_button,
        ):
            widget.setFixedHeight(control_height)
        self.open_button.setMinimumWidth(88)
        self.send_button.setMinimumWidth(78)
        self.port_combo.setFixedHeight(control_height)
        controls_grid.setColumnMinimumWidth(0, 76)
        controls_grid.setColumnStretch(4, 1)
        controls_grid.addWidget(serial_label, 0, 0)
        controls_grid.addWidget(self.dtr_button, 0, 1)
        controls_grid.addWidget(self.rts_button, 0, 2)
        controls_grid.addWidget(stm32_label, 1, 0)
        controls_grid.addWidget(reset_button, 1, 1)
        controls_grid.addWidget(boot_button, 1, 2)
        controls_grid.addWidget(xmodem_label, 2, 0)
        controls_grid.addWidget(self.xmodem_button, 2, 1)
        controls_grid.addWidget(self.cancel_xmodem_button, 2, 2)
        controls_grid.addWidget(self.transfer_progress, 2, 3)
        controls_grid.addWidget(self.transfer_label, 2, 4)
        controls_grid.addWidget(macro_label, 3, 0)
        controls_grid.addWidget(macro_button, 3, 1)
        controls_grid.addLayout(self.macro_layout, 3, 2, 1, 3)

        splitter = QSplitter(Qt.Vertical)
        splitter.setHandleWidth(5)
        splitter.setChildrenCollapsible(False)
        splitter.addWidget(self.output_tabs)
        splitter.addWidget(self.send_panel)
        splitter.setStretchFactor(0, 1)
        splitter.setStretchFactor(1, 0)
        splitter.setSizes([620, 140])
        layout = QVBoxLayout(self)
        layout.setContentsMargins(8, 6, 8, 6)
        layout.setSpacing(6)
        layout.addWidget(self.connection_panel)
        layout.addWidget(splitter, 1)
        layout.addWidget(self.controls_panel)
        QTimer.singleShot(0, self._balance_layout)

    def _balance_layout(self) -> None:
        """Give the output area the dominant share after layout negotiation."""
        if self.output_tabs.currentIndex() == 1:
            return
        available = self.findChild(QSplitter)
        if available is not None:
            height = max(240, available.height())
            send_height = max(70, self.send_panel.sizeHint().height())
            available.setSizes([max(210, height - send_height), send_height])

    def _set_terminal_cr_mode(self, enabled: bool) -> None:
        """Choose readable line-oriented output or raw terminal CR behavior."""
        self.terminal.cr_as_newline = enabled

    def _set_terminal_echo(self, enabled: bool) -> None:
        """Echo typed bytes locally for devices that do not echo input."""
        self.terminal.local_echo = enabled

    @pyqtSlot(bytes)
    def _send_terminal_input(self, data: bytes) -> None:
        """Send terminal keystrokes only while a serial port is open.

        A shell key such as Tab can be pressed before connecting.  Silently
        ignoring it avoids producing a modal write-error dialog for every
        keystroke and keeps focus inside the terminal workspace.
        """
        if not self._port_open:
            self._set_status("Open a serial port before typing")
            return
        self.send_requested.emit(data)

    def _update_terminal_size_label(self) -> None:
        self.terminal_size_label.setText(
            f"{self.terminal.emulator.columns} x {self.terminal.emulator.rows}"
        )

    def _set_terminal_search_query(self, query: str) -> None:
        self.terminal.search_query = query
        self.terminal._search_terminal()
        self._update_terminal_search_status()

    def _set_terminal_search_options(self) -> None:
        self.terminal.search_case_sensitive = self.terminal_search_case_check.isChecked()
        self.terminal.search_whole_word = self.terminal_search_word_check.isChecked()
        if self.terminal.search_query:
            self.terminal._search_terminal()
        self._update_terminal_search_status()

    def _terminal_search_next(self) -> None:
        self.terminal._find_terminal_next()
        self._update_terminal_search_status()

    def _terminal_search_previous(self) -> None:
        self.terminal._find_terminal_previous()
        self._update_terminal_search_status()

    def _update_terminal_search_status(self) -> None:
        if not self.terminal_search_edit.text():
            status = ""
        elif not self.terminal.search_matches:
            status = "未找到"
        elif self.terminal.search_index < 0:
            status = f"{len(self.terminal.search_matches)} 条结果"
        else:
            status = f"{self.terminal.search_index + 1} / {len(self.terminal.search_matches)}"
        self.terminal_search_status_label.setText(status)

    def _on_output_tab_changed(self, index: int) -> None:
        """Give Terminal the full vertical workspace when it is selected."""
        terminal_mode = index == 1
        self.connection_panel.setVisible(not terminal_mode)
        self.send_panel.setVisible(not terminal_mode)
        self.controls_panel.setVisible(not terminal_mode)
        if terminal_mode:
            self.terminal.refresh()
            self.terminal._invalidate_search()
            self.terminal.setFocus()
        else:
            self.receive.setFocus()

    def _rebuild_macros(self) -> None:
        while self.macro_layout.count():
            item = self.macro_layout.takeAt(0)
            if item.widget() is not None:
                item.widget().deleteLater()
        for macro in self._macros:
            button = QPushButton(str(macro.get("name", "Macro")))
            button.setToolTip(str(macro.get("command", "")))
            button.clicked.connect(lambda _checked=False, m=macro: self._send_macro(m))
            self.macro_layout.addWidget(button)
        self.macro_layout.addStretch()

    def set_devices(self, devices: list[DeviceInfo]) -> None:
        """Update the device list without discarding the selected path."""
        selected = self.port_combo.currentData()
        if selected:
            try:
                selected_path = str(Path(str(selected)).resolve())
            except OSError:
                selected_path = str(selected)
        else:
            selected_path = ""
        self._devices = devices
        self.port_combo.blockSignals(True)
        self.port_combo.clear()
        for device in devices:
            self.port_combo.addItem(device.display_name, device.path)
            index = self.port_combo.count() - 1
            if device.aliases:
                self.port_combo.setItemData(
                    index,
                    "稳定别名：\n" + "\n".join(device.aliases),
                    Qt.ToolTipRole,
                )
        if devices:
            self.port_combo.setEnabled(True)
            if self.open_button.text() == "打开":
                self.open_button.setEnabled(True)
        else:
            self.port_combo.addItem("未检测到可用串口设备", None)
            self.port_combo.setCurrentIndex(0)
            self.port_combo.setEnabled(False)
            if self.open_button.text() == "打开":
                self.open_button.setEnabled(False)
        if selected_path:
            index = self.port_combo.findData(selected_path)
            if index >= 0:
                self.port_combo.setCurrentIndex(index)
        self.port_combo.blockSignals(False)

    def _settings(self) -> dict:
        port = self.port_combo.currentData() or self.port_combo.currentText().strip()
        return {
            "port": str(port) if port else "",
            "baudrate": int(self.baud_combo.currentText() or 115200),
            "bytesize": int(self.bits_combo.currentText()),
            "stopbits": float(self.stop_combo.currentText()),
            "parity": self.parity_combo.currentData(),
            "flow": self.flow_combo.currentData(),
        }

    def _toggle_open(self) -> None:
        if self.open_button.text() == "打开":
            self._manual_close = False
            self._stop_reconnect()
            settings = self._settings()
            if not settings["port"]:
                self._on_error("No serial device selected")
                return
            self.open_button.setEnabled(False)
            self.open_requested.emit(settings)
        else:
            self.close_requested.emit()

    def _on_opened(self) -> None:
        self._port_open = True
        self._manual_close = False
        self._last_port_settings = self._settings()
        self._stop_reconnect()
        self.open_button.setEnabled(True)
        self.open_button.setText("关闭")
        self.open_button.setProperty("connected", True)
        self.open_button.style().unpolish(self.open_button)
        self.open_button.style().polish(self.open_button)

    def _on_closed(self) -> None:
        self._port_open = False
        self.open_button.setEnabled(True)
        self.open_button.setText("打开")
        self.open_button.setProperty("connected", False)
        self.open_button.style().unpolish(self.open_button)
        self.open_button.style().polish(self.open_button)
        if self.auto_reconnect_check.isChecked() and not self._manual_close and self._last_port_settings:
            self._reconnect_active = True
            self._reconnect_pending = False
            self._set_status("串口已断开，正在重连...")
            self._reconnect_timer.start()
        else:
            self._stop_reconnect()
            self._manual_close = False

    def _on_error(self, message: str) -> None:
        self.open_button.setEnabled(True)
        translated = self._translate_status(message)
        if self._reconnect_active or self._reconnect_pending:
            self._reconnect_pending = False
            self._set_status("串口已断开，正在重连...")
            return
        else:
            QMessageBox.warning(self, "串口助手", translated)

    def _set_auto_reconnect(self, enabled: bool) -> None:
        self.config.save_value("serial/auto_reconnect", enabled)
        if not enabled:
            self._stop_reconnect()
        elif not self._port_open and not self._manual_close and self._last_port_settings:
            self._reconnect_active = True
            self._set_status("串口已断开，正在重连...")
            self._reconnect_timer.start()

    def _try_reconnect(self) -> None:
        if (
            not self.auto_reconnect_check.isChecked()
            or self._port_open
            or self._manual_close
            or self._reconnect_pending
            or not self._last_port_settings
        ):
            return
        settings = self._last_port_settings
        port = str(settings.get("port", ""))
        if not port:
            return
        if not Path(port).exists():
            self._set_status("串口已断开，正在重连...")
            return
        self._reconnect_pending = True
        self.reconnect_requested.emit(settings)

    def _stop_reconnect(self) -> None:
        self._reconnect_timer.stop()
        self._reconnect_active = False
        self._reconnect_pending = False

    @staticmethod
    def _translate_status(message: str) -> str:
        """Translate worker and pyserial status text for the Chinese UI."""
        replacements = {
            "Port opened": "串口已打开",
            "Port closed": "串口已关闭",
            "Open failed:": "打开串口失败：",
            "Read failed:": "读取串口失败：",
            "Send failed:": "发送数据失败：",
            "DTR failed:": "DTR 控制失败：",
            "RTS failed:": "RTS 控制失败：",
            "STM32 reset sequence complete": "STM32 复位时序完成",
            "STM32 bootloader sequence complete": "STM32 Bootloader 时序完成",
            "STM32 sequence failed:": "STM32 时序失败：",
            "No serial device selected": "未选择串口设备",
            "Invalid HEX data": "HEX 数据格式错误",
            "HEX data must contain complete byte pairs": "HEX 数据必须由完整的字节对组成",
            "Save log failed:": "保存日志失败：",
            "Display throttled;": "显示已限流，跳过",
            "XModem transfer complete": "XModem 传输完成",
        }
        for source, target in replacements.items():
            if message.startswith(source):
                return target + message[len(source):]
        return message

    def _set_status(self, message: str) -> None:
        self._status_text = self._translate_status(message)
        self.status_changed.emit(self._status_text)

    def _set_display_mode(self, _index: int) -> None:
        mode = self.display_mode_combo.currentText()
        self.config.save_value("monitor/display_mode", mode)
        self._rx_offset = 0
        self.receive.clear()

    def _set_log_limit(self, value: int) -> None:
        self.receive.setMaximumBlockCount(value)

    def _search_flags(self, backward: bool = False) -> "QTextDocument.FindFlags":
        flags = QTextDocument.FindFlags()
        if self.search_case_check.isChecked():
            flags |= QTextDocument.FindCaseSensitively
        if self.search_word_check.isChecked():
            flags |= QTextDocument.FindWholeWords
        if backward:
            flags |= QTextDocument.FindBackward
        return flags

    def _find_next(self) -> None:
        query = self.search_edit.text()
        if not query:
            self.search_status_label.setText("未找到")
            return
        if self.receive.find(query, self._search_flags()):
            self.search_status_label.setText("已找到")
            return
        cursor = self.receive.textCursor()
        cursor.movePosition(QTextCursor.Start)
        self.receive.setTextCursor(cursor)
        self.search_status_label.setText("已找到" if self.receive.find(query, self._search_flags()) else "未找到")

    def _find_previous(self) -> None:
        query = self.search_edit.text()
        if not query:
            self.search_status_label.setText("未找到")
            return
        if self.receive.find(query, self._search_flags(backward=True)):
            self.search_status_label.setText("已找到")
            return
        cursor = self.receive.textCursor()
        cursor.movePosition(QTextCursor.End)
        self.receive.setTextCursor(cursor)
        found = self.receive.find(query, self._search_flags(backward=True))
        self.search_status_label.setText("已找到" if found else "未找到")

    def _show_search(self) -> None:
        if self.output_tabs.currentIndex() == 1:
            self.terminal_search_panel.setVisible(True)
            self.terminal_search_edit.setFocus()
            self.terminal_search_edit.selectAll()
        else:
            self.output_tabs.setCurrentIndex(0)
            self.search_panel.setVisible(True)
            self.search_edit.setFocus()
            self.search_edit.selectAll()

    def _hide_search(self) -> None:
        self.search_panel.setVisible(False)
        self.terminal_search_panel.setVisible(False)
        self.search_status_label.setText("")
        self.terminal_search_status_label.setText("")
        if self.output_tabs.currentIndex() == 1:
            self.terminal.setFocus()
        else:
            self.receive.setFocus()

    @pyqtSlot(bytes)
    def _on_data(self, data: bytes) -> None:
        """Queue incoming bytes for a throttled UI refresh.

        This slot is intentionally O(data size) with no widget operation.
        It is called by the serial worker's queued signal and must return
        quickly, otherwise a high-rate boot log can fill Qt's event queue.
        """
        if not data:
            return
        self._pending_rx.extend(data)
        if len(self._pending_rx) > self._pending_rx_limit:
            # A display cannot catch up with an unlimited stream.  Drop the
            # oldest not-yet-rendered bytes and reset the terminal parser on
            # the next flush so a cut ANSI sequence cannot corrupt the view.
            overflow = len(self._pending_rx) - self._pending_rx_limit
            del self._pending_rx[:overflow]
            self._display_dropped_bytes += overflow

    def _flush_received_data(self) -> None:
        """Render one bounded batch of queued serial data on the UI thread."""
        if not self._pending_rx:
            return

        if self._display_dropped_bytes:
            dropped = self._display_dropped_bytes
            self._display_dropped_bytes = 0
            self.terminal.clear()
            self._set_status(f"Display throttled; {dropped:,} bytes skipped")

        data = bytes(self._pending_rx[:self._rx_flush_limit])
        del self._pending_rx[:len(data)]
        self.terminal.write_bytes(data, render=self.output_tabs.currentIndex() == 1)
        if self.pause_button.isChecked():
            return
        display_mode = self.display_mode_combo.currentText()
        if display_mode == "HEX":
            lines = []
            for index in range(0, len(data), 16):
                chunk = data[index:index + 16]
                lines.append("RX  " + " ".join(f"{byte:02X}" for byte in chunk))
                self._rx_offset += len(chunk)
            text = "\n".join(lines)
        elif display_mode == "ASCII + HEX":
            hex_text = " ".join(f"{byte:02X}" for byte in data)
            ascii_text = data.decode("utf-8", errors="replace")
            text = f"RX\nASCII:  {ascii_text}\nHEX:    {hex_text}"
        else:
            text = data.decode("utf-8", errors="replace")
        if self.timestamp_check.isChecked():
            now = dt.datetime.now().strftime("%H:%M:%S.%f")[:-3]
            text = f"[{now}] {text}"
        self.receive.appendPlainText(text)
        if self.autoscroll_check.isChecked():
            scrollbar = self.receive.verticalScrollBar()
            scrollbar.setValue(scrollbar.maximum())

    def _reset_display(self) -> None:
        self._rx_offset = 0
        self.receive.clear()

    def _clear_receive(self) -> None:
        self.receive.clear()
        self._rx_offset = 0

    def _parse_send(self) -> bytes:
        value = self.send_edit.text()
        if self.send_mode.currentText() == "HEX":
            compact = "".join(value.split())
            if len(compact) % 2:
                raise ValueError("HEX data must contain complete byte pairs")
            try:
                data = bytes.fromhex(compact)
            except ValueError as exc:
                raise ValueError("Invalid HEX data") from exc
        else:
            data = value.encode("utf-8")
        return data + bytes(self.suffix_combo.currentData() or b"")

    def _send_current(self) -> None:
        try:
            data = self._parse_send()
        except ValueError as exc:
            self._on_error(str(exc))
            return
        if not data:
            return
        text = self.send_edit.text()
        if text and text not in self._history:
            self._history.insert(0, text)
            self._history = self._history[:30]
            self.history_combo.clear()
            self.history_combo.addItems(self._history)
        self.send_requested.emit(data)

    def _update_loop(self) -> None:
        try:
            data = self._parse_send() if self.loop_check.isChecked() else b""
        except ValueError:
            data = b""
        self.loop_requested.emit(data, self.loop_interval.value())

    def _send_macro(self, macro: dict) -> None:
        self.send_edit.setText(str(macro.get("command", "")))
        self.send_mode.setCurrentText("HEX" if macro.get("hex") else "ASCII")
        self._send_current()

    def _edit_macros(self) -> None:
        dialog = MacroDialog(self._macros, self)
        if dialog.exec_():
            self._macros = dialog.macros()
            self.config.save_macros(self._macros)
            self._rebuild_macros()

    def _update_statistics(self, rx: int, tx: int) -> None:
        self._rx_count = rx
        self._tx_count = tx
        self.statistics_changed.emit(rx, tx)

    def _update_line_states(self, dtr: bool, rts: bool) -> None:
        self.dtr_button.blockSignals(True)
        self.rts_button.blockSignals(True)
        self.dtr_button.setChecked(dtr)
        self.rts_button.setChecked(rts)
        self.dtr_button.setText(f"DTR {'高电平' if dtr else '低电平'}")
        self.rts_button.setText(f"RTS {'高电平' if rts else '低电平'}")
        self.dtr_button.blockSignals(False)
        self.rts_button.blockSignals(False)

    def _save_log(self) -> None:
        filename, _ = QFileDialog.getSaveFileName(self, "保存接收日志", "serial.log", "日志文件 (*.log);;所有文件 (*)")
        if filename:
            try:
                Path(filename).write_text(self.receive.toPlainText(), encoding="utf-8")
            except OSError as exc:
                self._on_error(f"Save log failed: {exc}")

    def _choose_xmodem_file(self) -> None:
        filename, _ = QFileDialog.getOpenFileName(self, "选择 XModem 文件")
        if not filename:
            return
        self.transfer_progress.setValue(0)
        self.xmodem_button.setEnabled(False)
        self.cancel_xmodem_button.setEnabled(True)
        self.xmodem_requested.emit(filename)

    def _transfer_progress(self, sent: int, total: int, speed: float) -> None:
        self.transfer_progress.setValue(int(sent * 100 / total) if total else 0)
        self.transfer_label.setText(f"{sent:,}/{total:,} 字节  {speed / 1024:.1f} KiB/s")

    def _transfer_finished(self, success: bool, message: str) -> None:
        self.xmodem_button.setEnabled(True)
        self.cancel_xmodem_button.setEnabled(False)
        self.transfer_label.setText("传输完成" if success else f"传输失败：{message}")
        if not success:
            QMessageBox.warning(self, "XModem 传输", message)

    def shutdown(self) -> None:
        """Stop the worker event loop cleanly during application shutdown."""
        self._rx_flush_timer.stop()
        self._reconnect_timer.stop()
        self.cancel_transfer.emit()
        self.stop_requested.emit()
        self.thread.wait(2000)
