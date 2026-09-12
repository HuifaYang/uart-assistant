"""Application entry point.

Run from the project directory with ``python3 main.py``.  The offscreen
platform is intentionally not forced here, so the same entry point works with
normal Linux desktop sessions and embedded X11/Wayland environments.
"""

from __future__ import annotations

import sys
import signal

from PyQt5.QtCore import QTimer
from PyQt5.QtGui import QFont, QPalette, QColor
from PyQt5.QtWidgets import QApplication

from view.main_window import MainWindow


def apply_dark_theme(app: QApplication) -> None:
    """Apply a clean, light palette inspired by Apple's desktop tools."""
    palette = QPalette()
    palette.setColor(QPalette.Window, QColor("#f5f5f7"))
    palette.setColor(QPalette.WindowText, QColor("#1d1d1f"))
    palette.setColor(QPalette.Base, QColor("#ffffff"))
    palette.setColor(QPalette.AlternateBase, QColor("#f5f5f7"))
    palette.setColor(QPalette.ToolTipBase, QColor("#ffffff"))
    palette.setColor(QPalette.ToolTipText, QColor("#1d1d1f"))
    palette.setColor(QPalette.Text, QColor("#1d1d1f"))
    palette.setColor(QPalette.Button, QColor("#ffffff"))
    palette.setColor(QPalette.ButtonText, QColor("#1d1d1f"))
    palette.setColor(QPalette.BrightText, QColor("#ffffff"))
    palette.setColor(QPalette.Highlight, QColor("#007aff"))
    palette.setColor(QPalette.HighlightedText, QColor("#ffffff"))
    app.setPalette(palette)
    app.setStyleSheet(
        """
        QMainWindow, QWidget { color: #1d1d1f; font-family: "Noto Sans CJK SC", "Microsoft YaHei", sans-serif; }
        QStatusBar { background: #fafafa; color: #6e6e73; border-top: 1px solid #d9d9de; padding: 2px 8px; }
        QStatusBar QLabel { padding: 1px 8px; }
        QLabel#statusIndicator { color: #6e6e73; font-weight: 500; }
        QLabel#statusIndicator[connected="true"] { color: #00875a; }
        QLabel#statusMetric { color: #6e6e73; }
        QLabel#deviceStatus { color: #6e6e73; }
        QLabel#connectionDetail { color: #6e6e73; }
        QWidget#toolbar { background: #fafafa; border-bottom: 1px solid #e4e4e9; }
        QWidget#contentPanel { background: #ffffff; border: 1px solid #e4e4e9; }
        QLabel#fieldLabel { color: #75757d; font-size: 9px; }
        QLabel#sectionLabel { color: #75757d; font-size: 9px; font-weight: 500; }
        QFrame#divider { color: #e4e4e9; background: #e4e4e9; width: 1px; }
        QPlainTextEdit#monitorView { font-family: "DejaVu Sans Mono", "JetBrains Mono", monospace; }
        QPushButton { background: #ffffff; border: 1px solid #cfcfd5; border-radius: 4px; padding: 4px 9px; }
        QPushButton:hover { background: #f4f4f6; border-color: #a9a9b2; }
        QPushButton:pressed { background: #e9e9ee; }
        QPushButton:checked { background: #eceff7; border-color: #7a86a8; color: #2f4570; }
        QPushButton#primaryButton { background: #2464d6; border-color: #1d54b6; color: #ffffff; font-weight: 600; }
        QPushButton#primaryButton:hover { background: #1f57bd; }
        QPushButton#primaryButton:pressed { background: #1b4ba7; }
        QPushButton#newTabButton { border: 1px solid #cfcfd5; background: #ffffff; padding: 2px 8px; }
        QPlainTextEdit, QLineEdit, QComboBox, QSpinBox { background: #ffffff; border: 1px solid #cfcfd5; border-radius: 4px; padding: 3px 6px; selection-background-color: #2464d6; }
        QPlainTextEdit:focus, QLineEdit:focus, QComboBox:focus, QSpinBox:focus { border-color: #2464d6; }
        QComboBox QAbstractItemView { background: #ffffff; border: 1px solid #d2d2d7; selection-background-color: #007aff; }
        QTabWidget#mainTabs::pane { border: 0; background: #f5f5f7; }
        QTabWidget#outputTabs::pane { border: 1px solid #e4e4e9; border-radius: 0; background: #ffffff; }
        QTabBar { qproperty-drawBase: 0; }
        QTabBar::tab { background: transparent; color: #6e6e73; padding: 5px 14px; border: 0; }
        QTabBar::tab:selected { color: #1d1d1f; font-weight: 600; border-bottom: 2px solid #2464d6; }
        QTabBar::tab:hover:!selected { color: #1d1d1f; background: #efeff2; }
        QProgressBar { background: #f0f0f2; border: 1px solid #d9d9de; border-radius: 2px; text-align: center; }
        QProgressBar::chunk { background: #2464d6; }
        QLabel#mutedLabel { color: #6e6e73; }
        QSplitter::handle { background: #e4e4e9; height: 4px; }
        QCheckBox { spacing: 6px; }
        QToolTip { background: #1d1d1f; color: #ffffff; border: 0; padding: 5px 8px; }
        """
    )


def main() -> int:
    app = QApplication(sys.argv)
    # The desktop shell uses this value for taskbar grouping/title rendering.
    # Keep it ASCII to avoid panel-side locale/font mojibake.
    app.setApplicationName("UART Assistant")
    app.setOrganizationName("UARTAssistant")
    app.setFont(QFont("Noto Sans CJK SC", 10))
    apply_dark_theme(app)
    window = MainWindow()

    # Python signal handlers run when the interpreter regains control.  A
    # small Qt timer provides that opportunity while exec_() is active.
    def handle_sigint(_signum, _frame) -> None:
        window.close()
        app.quit()

    signal.signal(signal.SIGINT, handle_sigint)
    app._sigint_pump = QTimer(app)
    app._sigint_pump.setInterval(100)
    app._sigint_pump.timeout.connect(lambda: None)
    app._sigint_pump.start()
    window.show()
    return app.exec_()


if __name__ == "__main__":
    raise SystemExit(main())
