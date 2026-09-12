"""Small VT100/ANSI terminal state machine for the serial console view.

This is intentionally a screen emulator rather than a text formatter.  A
Linux shell frequently moves the cursor, erases part of a line, redraws a
prompt, or switches to an alternate screen.  Keeping a fixed cell buffer is
what makes those operations behave like a terminal application such as
MobaXterm instead of accumulating control sequences in a text box.
"""

from __future__ import annotations

import codecs
from collections import deque
from dataclasses import dataclass


DEFAULT_FG = "#d7e2ea"
DEFAULT_BG = "#11161b"

ANSI_COLORS = {
    0: "#20252b", 1: "#ff6b6b", 2: "#62d69b", 3: "#ffd166",
    4: "#62a8ff", 5: "#d18cff", 6: "#5ed9e8", 7: "#e6e9ed",
    8: "#7d8792", 9: "#ff8585", 10: "#8ae6b4", 11: "#ffe08a",
    12: "#8fc4ff", 13: "#e0aeff", 14: "#8eeaf2", 15: "#ffffff",
}


@dataclass
class Cell:
    """One terminal cell, including the attributes used to render it."""

    char: str = " "
    foreground: str = DEFAULT_FG
    background: str = DEFAULT_BG
    bold: bool = False
    reverse: bool = False


class TerminalEmulator:
    """A practical VT100/ANSI subset suitable for serial Linux consoles."""

    def __init__(self, columns: int = 120, rows: int = 32, scrollback: int = 2000) -> None:
        self.scrollback_limit = scrollback
        self.columns = max(20, columns)
        self.rows = max(5, rows)
        self.scrollback: deque[list[Cell]] = deque(maxlen=scrollback)
        # A real terminal treats CR as carriage-return only.  This matters
        # for readline: shells use a bare CR to redraw the current command
        # line (for example after Up/Down history navigation), then use ANSI
        # erase sequences before writing the replacement text.
        self.cr_as_newline = False
        self._decoder = codecs.getincrementaldecoder("utf-8")(errors="replace")
        self._alternate_buffer: list[list[Cell]] | None = None
        self._alternate_cursor: tuple[int, int] | None = None
        self.reset()

    def _blank_cell(self) -> Cell:
        return Cell()

    def _blank_line(self) -> list[Cell]:
        return [self._blank_cell() for _ in range(self.columns)]

    def reset(self) -> None:
        """Reset the screen, cursor, parser and active text attributes."""
        self.buffer = [self._blank_line() for _ in range(self.rows)]
        self.scrollback.clear()
        self.cursor_x = 0
        self.cursor_y = 0
        self.saved_cursor = (0, 0)
        self.scroll_top = 0
        self.scroll_bottom = self.rows - 1
        self.cursor_visible = True
        self.foreground = DEFAULT_FG
        self.background = DEFAULT_BG
        self.bold = False
        self.reverse = False
        self._state = "normal"
        self._csi = ""
        self._osc = ""
        self._osc_esc = False
        self._skip_lf = False
        self._alternate_buffer = None
        self._alternate_cursor = None

    def resize(self, columns: int, rows: int) -> None:
        """Resize the grid while retaining visible content and cursor position."""
        columns = max(20, int(columns))
        rows = max(5, int(rows))
        if columns == self.columns and rows == self.rows:
            return
        old_buffer = self.buffer
        old_columns = self.columns
        self.columns, self.rows = columns, rows
        self.buffer = [self._blank_line() for _ in range(rows)]
        for y, old_line in enumerate(old_buffer[:rows]):
            self.buffer[y][: min(old_columns, columns)] = old_line[: min(old_columns, columns)]
        self.cursor_x = min(self.cursor_x, columns - 1)
        self.cursor_y = min(self.cursor_y, rows - 1)
        self.scroll_top = 0
        self.scroll_bottom = rows - 1

    def feed(self, data: bytes) -> None:
        """Consume serial bytes, including split UTF-8 and ANSI sequences."""
        for char in self._decoder.decode(data):
            self._consume(char)

    def _consume(self, char: str) -> None:
        if self._state == "csi":
            self._csi += char
            if "@" <= char <= "~":
                self._apply_csi(self._csi)
                self._state = "normal"
                self._csi = ""
            elif len(self._csi) > 64:
                self._state = "normal"
                self._csi = ""
            return

        if self._state == "osc":
            if char == "\a":
                self._state = "normal"
                self._osc = ""
            elif self._osc_esc and char == "\\":
                self._state = "normal"
                self._osc = ""
                self._osc_esc = False
            else:
                self._osc_esc = char == "\x1b"
                if not self._osc_esc:
                    self._osc += char
            return

        if self._state == "escape":
            if char == "[":
                self._state = "csi"
                self._csi = ""
            elif char == "]":
                self._state = "osc"
                self._osc = ""
                self._osc_esc = False
            elif char == "7":
                self.saved_cursor = (self.cursor_x, self.cursor_y)
                self._state = "normal"
            elif char == "8":
                self.cursor_x, self.cursor_y = self.saved_cursor
                self._clamp_cursor()
                self._state = "normal"
            elif char == "D":
                self._line_feed()
                self._state = "normal"
            elif char == "M":
                self._reverse_line_feed()
                self._state = "normal"
            elif char == "c":
                self.reset()
            else:
                self._state = "normal"
            return

        if self._skip_lf:
            self._skip_lf = False
            if char == "\n":
                return

        if char == "\x1b":
            self._state = "escape"
        elif char == "\r":
            self.cursor_x = 0
            if self.cr_as_newline:
                self._line_feed()
                self._skip_lf = True
        elif char == "\n":
            self._line_feed()
        elif char == "\b":
            self.cursor_x = max(0, self.cursor_x - 1)
        elif char == "\t":
            self.cursor_x = min(self.columns - 1, ((self.cursor_x // 8) + 1) * 8)
        elif char == "\a":
            pass
        elif ord(char) >= 32 and char != "\x7f":
            self._put(char)

    def _clamp_cursor(self) -> None:
        self.cursor_x = max(0, min(self.columns - 1, self.cursor_x))
        self.cursor_y = max(0, min(self.rows - 1, self.cursor_y))

    def _put(self, char: str) -> None:
        if self.cursor_x >= self.columns:
            self.cursor_x = 0
            self._line_feed()
        self.buffer[self.cursor_y][self.cursor_x] = Cell(
            char=char,
            foreground=self.foreground,
            background=self.background,
            bold=self.bold,
            reverse=self.reverse,
        )
        self.cursor_x += 1

    def _line_feed(self) -> None:
        if self.cursor_y < self.scroll_bottom:
            self.cursor_y += 1
            return
        removed = self.buffer.pop(self.scroll_top)
        self.buffer.insert(self.scroll_bottom, self._blank_line())
        if self.scroll_top == 0:
            self.scrollback.append(removed)

    def _reverse_line_feed(self) -> None:
        if self.cursor_y > self.scroll_top:
            self.cursor_y -= 1
        else:
            self.buffer.pop(self.scroll_bottom)
            self.buffer.insert(self.scroll_top, self._blank_line())

    @staticmethod
    def _params(raw: str) -> tuple[bool, list[int]]:
        private = raw.startswith("?")
        if private:
            raw = raw[1:]
        if not raw:
            return private, []
        values = []
        for value in raw.split(";"):
            try:
                values.append(int(value or 0))
            except ValueError:
                values.append(0)
        return private, values

    def _apply_csi(self, sequence: str) -> None:
        command = sequence[-1]
        private, values = self._params(sequence[:-1])
        # Most CSI movement/count commands default to 1, while the erase
        # commands J/K default to mode 0 when their parameter is omitted.
        first = values[0] if values else (0 if command in ("J", "K") else 1)
        if command == "A":
            self.cursor_y = max(self.scroll_top, self.cursor_y - first)
        elif command == "B":
            self.cursor_y = min(self.scroll_bottom, self.cursor_y + first)
        elif command == "C" or command == "a":
            self.cursor_x = min(self.columns - 1, self.cursor_x + first)
        elif command == "D":
            self.cursor_x = max(0, self.cursor_x - first)
        elif command in ("G", "`"):
            self.cursor_x = max(0, min(self.columns - 1, first - 1))
        elif command in ("H", "f"):
            row = values[0] if values else 1
            col = values[1] if len(values) > 1 else 1
            self.cursor_y = max(0, min(self.rows - 1, row - 1))
            self.cursor_x = max(0, min(self.columns - 1, col - 1))
        elif command == "d":
            self.cursor_y = max(0, min(self.rows - 1, first - 1))
        elif command == "J":
            self._erase_display(first)
        elif command == "K":
            self._erase_line(first)
        elif command == "m":
            self._sgr(values)
        elif command == "s":
            self.saved_cursor = (self.cursor_x, self.cursor_y)
        elif command == "u":
            self.cursor_x, self.cursor_y = self.saved_cursor
            self._clamp_cursor()
        elif command == "r":
            top = (values[0] if values else 1) - 1
            bottom = (values[1] if len(values) > 1 else self.rows) - 1
            if 0 <= top < bottom < self.rows:
                self.scroll_top, self.scroll_bottom = top, bottom
                self.cursor_x = self.cursor_y = 0
        elif command == "h" and private:
            self._set_private_mode(values, True)
        elif command == "l" and private:
            self._set_private_mode(values, False)
        elif command == "@":
            self._insert_chars(first)
        elif command == "P":
            self._delete_chars(first)
        elif command == "L":
            self._insert_lines(first)
        elif command == "M":
            self._delete_lines(first)

    def _set_private_mode(self, values: list[int], enabled: bool) -> None:
        if 25 in values:
            self.cursor_visible = enabled
        if 1049 in values:
            if enabled:
                self._alternate_buffer = self.buffer
                self._alternate_cursor = (self.cursor_x, self.cursor_y)
                self.buffer = [self._blank_line() for _ in range(self.rows)]
                self.cursor_x = self.cursor_y = 0
            elif self._alternate_buffer is not None:
                self.buffer = self._alternate_buffer
                self.cursor_x, self.cursor_y = self._alternate_cursor or (0, 0)
                self._alternate_buffer = None

    def _erase_display(self, mode: int) -> None:
        if mode == 2 or mode == 3:
            self.buffer = [self._blank_line() for _ in range(self.rows)]
        elif mode == 1:
            for y in range(self.cursor_y + 1):
                start = self.cursor_x + 1 if y == self.cursor_y else 0
                self.buffer[y][start:] = [self._blank_cell() for _ in range(self.columns - start)]
        else:
            for y in range(self.cursor_y, self.rows):
                start = self.cursor_x if y == self.cursor_y else 0
                self.buffer[y][start:] = [self._blank_cell() for _ in range(self.columns - start)]

    def _erase_line(self, mode: int) -> None:
        if mode == 2:
            start, end = 0, self.columns
        elif mode == 1:
            start, end = 0, self.cursor_x + 1
        else:
            start, end = self.cursor_x, self.columns
        self.buffer[self.cursor_y][start:end] = [self._blank_cell() for _ in range(end - start)]

    def _insert_chars(self, count: int) -> None:
        line = self.buffer[self.cursor_y]
        count = min(count, self.columns - self.cursor_x)
        line[self.cursor_x:] = ([self._blank_cell() for _ in range(count)] + line[self.cursor_x:self.columns - count])

    def _delete_chars(self, count: int) -> None:
        line = self.buffer[self.cursor_y]
        count = min(count, self.columns - self.cursor_x)
        line[self.cursor_x:] = line[self.cursor_x + count:] + [self._blank_cell() for _ in range(count)]

    def _insert_lines(self, count: int) -> None:
        count = min(count, self.scroll_bottom - self.cursor_y + 1)
        for _ in range(count):
            self.buffer.insert(self.cursor_y, self._blank_line())
            self.buffer.pop(self.scroll_bottom + 1)

    def _delete_lines(self, count: int) -> None:
        count = min(count, self.scroll_bottom - self.cursor_y + 1)
        for _ in range(count):
            self.buffer.pop(self.cursor_y)
            self.buffer.insert(self.scroll_bottom, self._blank_line())

    def _sgr(self, values: list[int]) -> None:
        values = values or [0]
        index = 0
        while index < len(values):
            value = values[index]
            if value == 0:
                self.foreground, self.background = DEFAULT_FG, DEFAULT_BG
                self.bold = self.reverse = False
            elif value == 1:
                self.bold = True
            elif value == 22:
                self.bold = False
            elif value == 7:
                self.reverse = True
            elif value == 27:
                self.reverse = False
            elif value == 39:
                self.foreground = DEFAULT_FG
            elif value == 49:
                self.background = DEFAULT_BG
            elif 30 <= value <= 37:
                self.foreground = ANSI_COLORS[value - 30]
            elif 90 <= value <= 97:
                self.foreground = ANSI_COLORS[value - 90 + 8]
            elif 40 <= value <= 47:
                self.background = ANSI_COLORS[value - 40]
            elif 100 <= value <= 107:
                self.background = ANSI_COLORS[value - 100 + 8]
            elif value in (38, 48) and index + 2 < len(values) and values[index + 1] == 5:
                color = ANSI_COLORS.get(values[index + 2] % 16, DEFAULT_FG)
                if value == 38:
                    self.foreground = color
                else:
                    self.background = color
                index += 2
            index += 1

    def visible_rows(self) -> list[list[Cell]]:
        """Return scrollback followed by the current fixed-size screen."""
        return list(self.scrollback) + self.buffer
