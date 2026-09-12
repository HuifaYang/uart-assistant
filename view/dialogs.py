"""Small dialogs kept separate from the main widgets."""

from __future__ import annotations

from typing import Any

from PyQt5.QtWidgets import (
    QDialog,
    QDialogButtonBox,
    QFormLayout,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QPushButton,
    QWidget,
    QVBoxLayout,
    QHBoxLayout,
)


class MacroDialog(QDialog):
    """Edit the label and command of the reusable macro button panel."""

    def __init__(self, macros: list[dict[str, Any]], parent=None) -> None:
        super().__init__(parent)
        self.setWindowTitle("编辑宏命令")
        self.resize(520, 340)
        self.list = QListWidget()
        self._load(macros)
        add = QPushButton("添加")
        remove = QPushButton("删除")
        add.clicked.connect(lambda: self._add_row("新宏命令", ""))
        remove.clicked.connect(self._remove_row)
        buttons = QHBoxLayout()
        buttons.addWidget(add)
        buttons.addWidget(remove)
        buttons.addStretch()
        box = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        box.accepted.connect(self.accept)
        box.rejected.connect(self.reject)
        layout = QVBoxLayout(self)
        layout.addWidget(self.list)
        layout.addLayout(buttons)
        layout.addWidget(box)

    def _load(self, macros: list[dict[str, Any]]) -> None:
        for macro in macros:
            self._add_row(str(macro.get("name", "Macro")), str(macro.get("command", "")))

    def _add_row(self, name: str, command: str) -> None:
        item = QListWidgetItem(self.list)
        editor = _MacroRow(name, command)
        item.setSizeHint(editor.sizeHint())
        self.list.setItemWidget(item, editor)

    def _remove_row(self) -> None:
        row = self.list.currentRow()
        if row >= 0:
            self.list.takeItem(row)

    def macros(self) -> list[dict[str, Any]]:
        result = []
        for index in range(self.list.count()):
            editor = self.list.itemWidget(self.list.item(index))
            if editor is not None:
                result.append({"name": editor.name.text().strip() or "Macro", "command": editor.command.text(), "hex": False})
        return result


class _MacroRow(QWidget):
    """Widget containing the two editable fields for one macro."""

    def __init__(self, name: str, command: str) -> None:
        super().__init__()
        layout = QHBoxLayout(self)
        self.name = QLineEdit(name)
        self.command = QLineEdit(command)
        self.name.setPlaceholderText("按钮名称")
        self.command.setPlaceholderText("命令内容")
        layout.addWidget(self.name)
        layout.addWidget(self.command, 1)
