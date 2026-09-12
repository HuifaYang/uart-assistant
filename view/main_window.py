"""Main application window and shared hotplug monitor."""

from __future__ import annotations

from PyQt5.QtCore import Qt, QThread
from PyQt5.QtWidgets import QMainWindow, QTabWidget, QLabel, QPushButton

from controller.hotplug import HotplugMonitor
from model.config import ConfigManager
from model.serial_port import DeviceInfo
from view.serial_widget import SerialWidget


class MainWindow(QMainWindow):
    """Host multiple independent serial widgets in tabs."""

    def __init__(self) -> None:
        super().__init__()
        # Keep the WM/taskbar title ASCII-only.  Some Ubuntu panel/font
        # combinations render non-ASCII X11 window titles as mojibake; the
        # application content and toolbar remain fully Chinese.
        self.setWindowTitle("UART Assistant")
        self.resize(1360, 900)
        self.config = ConfigManager()
        self.tabs = QTabWidget()
        self.tabs.setObjectName("mainTabs")
        self.tabs.setTabsClosable(True)
        self.tabs.setDocumentMode(True)
        self.tabs.tabCloseRequested.connect(self.close_tab)
        self.tabs.currentChanged.connect(lambda _index: self._sync_status())
        self.setCentralWidget(self.tabs)
        new_button = QPushButton("＋ 新建串口")
        new_button.setObjectName("newTabButton")
        new_button.clicked.connect(self.add_tab)
        self.tabs.setCornerWidget(new_button, Qt.TopRightCorner)

        self.connection_status_label = QLabel("● 未连接")
        self.connection_status_label.setObjectName("statusIndicator")
        self.rx_status_label = QLabel("RX 0 B")
        self.rx_status_label.setObjectName("statusMetric")
        self.tx_status_label = QLabel("TX 0 B")
        self.tx_status_label.setObjectName("statusMetric")
        self.connection_detail_label = QLabel("")
        self.connection_detail_label.setObjectName("connectionDetail")
        self.device_status_label = QLabel("检测到 0 个串口设备")
        self.device_status_label.setObjectName("deviceStatus")
        status_bar = self.statusBar()
        status_bar.setSizeGripEnabled(False)
        status_bar.addWidget(self.connection_status_label)
        status_bar.addWidget(self.rx_status_label)
        status_bar.addWidget(self.tx_status_label)
        status_bar.addWidget(self.connection_detail_label)
        status_bar.addPermanentWidget(self.device_status_label)

        self.hotplug_thread = QThread(self)
        self.hotplug = HotplugMonitor()
        self.hotplug.moveToThread(self.hotplug_thread)
        self.hotplug_thread.started.connect(self.hotplug.start)
        self.hotplug.devices_changed.connect(self._devices_changed)
        self.hotplug_thread.start()
        self.add_tab()

    def add_tab(self) -> None:
        widget = SerialWidget(self.config)
        widget.request_scan.connect(self.hotplug.scan)
        widget.status_changed.connect(lambda message, target=widget: self._on_session_status(target, message))
        widget.statistics_changed.connect(lambda rx, tx, target=widget: self._on_session_statistics(target, rx, tx))
        self.tabs.addTab(widget, "串口 %d" % (self.tabs.count() + 1))
        self.tabs.setCurrentWidget(widget)
        # The monitor may have scanned before this tab existed.
        self.hotplug.scan_requested.emit()

    def _sync_status(self) -> None:
        widget = self.tabs.currentWidget()
        if isinstance(widget, SerialWidget):
            self.connection_status_label.setText("● 已连接" if widget.connected else "● 未连接")
            self.connection_detail_label.setText(widget.connection_summary() if widget.connected else "")
            self.connection_detail_label.setVisible(bool(self.connection_detail_label.text()))
            rx, tx = widget.statistics
            self.rx_status_label.setText(f"RX {rx:,} B")
            self.tx_status_label.setText(f"TX {tx:,} B")
        else:
            self.connection_status_label.setText("● 未连接")
            self.connection_detail_label.setText("")
            self.connection_detail_label.setVisible(False)
            self.rx_status_label.setText("RX 0 B")
            self.tx_status_label.setText("TX 0 B")
        connected = bool(widget and widget.connected)
        self.connection_status_label.setProperty("connected", connected)
        self.connection_status_label.style().unpolish(self.connection_status_label)
        self.connection_status_label.style().polish(self.connection_status_label)

    def _on_session_status(self, widget: SerialWidget, message: str) -> None:
        if self.tabs.currentWidget() is widget:
            self.connection_status_label.setText("● 已连接" if widget.is_open else "● 未连接")
            self.connection_detail_label.setText(widget.connection_summary() if widget.is_open else "")
            self.connection_detail_label.setVisible(bool(self.connection_detail_label.text()))
            connected = widget.is_open
            self.connection_status_label.setProperty("connected", connected)
            self.connection_status_label.style().unpolish(self.connection_status_label)
            self.connection_status_label.style().polish(self.connection_status_label)

    def _on_session_statistics(self, widget: SerialWidget, rx: int, tx: int) -> None:
        if self.tabs.currentWidget() is widget:
            self.rx_status_label.setText(f"RX {rx:,} B")
            self.tx_status_label.setText(f"TX {tx:,} B")

    def _devices_changed(self, devices: list[DeviceInfo]) -> None:
        for index in range(self.tabs.count()):
            tab = self.tabs.widget(index)
            if isinstance(tab, SerialWidget):
                tab.set_devices(devices)
        self.device_status_label.setText(f"检测到 {len(devices)} 个串口设备")

    def close_tab(self, index: int) -> None:
        if self.tabs.count() <= 1:
            return
        widget = self.tabs.widget(index)
        self.tabs.removeTab(index)
        if isinstance(widget, SerialWidget):
            widget.shutdown()
            widget.deleteLater()

    def closeEvent(self, event) -> None:  # noqa: N802 - Qt API name
        for index in range(self.tabs.count()):
            widget = self.tabs.widget(index)
            if isinstance(widget, SerialWidget):
                widget.shutdown()
        self.hotplug.stop_requested.emit()
        self.hotplug_thread.wait(2000)
        event.accept()
