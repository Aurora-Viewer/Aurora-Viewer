#!/usr/bin/env python3
"""Runs the console fixture of test-tools.ps1 on a pseudo-terminal (Linux).

The fixture needs a console it can read and resize, which .NET has only on
Windows. This is that console: a small terminal model answering the cursor
position queries, following `stty` resizes, and writing its screen to a file
when the fixture asks with the private sequence ESC ] 9999 ; dump BEL.

    test-tools-console.py <screen file> <command> [arguments...]
"""
import codecs
import fcntl
import os
import pty
import re
import select
import struct
import sys
import termios
import time

TIMEOUT_SECONDS = 120
CSI = re.compile(r"\x1b\[([?>=]?)([0-9;]*)([ -/]*)([@-~])")
OSC = re.compile(r"\x1b\](.*?)(?:\x07|\x1b\\)", re.S)


class Screen:
    def __init__(self, cols, rows):
        self.cols, self.rows = cols, rows
        self.grid = [[" "] * cols for _ in range(rows)]
        self.x = self.y = 0
        self.cursor_shown = True

    def resize(self, cols, rows):
        grid = [[" "] * cols for _ in range(rows)]
        for y in range(min(rows, self.rows)):
            for x in range(min(cols, self.cols)):
                grid[y][x] = self.grid[y][x]
        self.grid, self.cols, self.rows = grid, cols, rows
        self.x, self.y = min(self.x, cols - 1), min(self.y, rows - 1)

    def line_feed(self):
        if self.y == self.rows - 1:
            self.grid.pop(0)
            self.grid.append([" "] * self.cols)
        else:
            self.y += 1

    def put(self, ch):
        # the wrap waits for the next character, as in xterm
        if self.x >= self.cols:
            self.x = 0
            self.line_feed()
        self.grid[self.y][self.x] = ch
        self.x += 1

    def erase(self, y, start, end):
        for x in range(start, end):
            self.grid[y][x] = " "

    def csi(self, private, params, final, reply):
        nums = [int(p) if p else 0 for p in params.split(";")] if params else []
        first = nums[0] if nums else 0
        count = max(first, 1)
        if private == "?":
            if first == 25 and final in "hl":
                self.cursor_shown = final == "h"
        elif final in "Hf":
            row = nums[0] if nums else 1
            col = nums[1] if len(nums) > 1 else 1
            self.y = min(max(row, 1), self.rows) - 1
            self.x = min(max(col, 1), self.cols) - 1
        elif final == "A":
            self.y = max(self.y - count, 0)
        elif final == "B":
            self.y = min(self.y + count, self.rows - 1)
        elif final == "C":
            self.x = min(self.x + count, self.cols - 1)
        elif final == "D":
            self.x = max(min(self.x, self.cols - 1) - count, 0)
        elif final == "G":
            self.x = min(count, self.cols) - 1
        elif final == "d":
            self.y = min(count, self.rows) - 1
        elif final == "J":
            if first == 0:
                self.erase(self.y, min(self.x, self.cols), self.cols)
                for y in range(self.y + 1, self.rows):
                    self.erase(y, 0, self.cols)
            elif first == 1:
                self.erase(self.y, 0, min(self.x + 1, self.cols))
                for y in range(self.y):
                    self.erase(y, 0, self.cols)
            else:
                for y in range(self.rows):
                    self.erase(y, 0, self.cols)
        elif final == "K":
            if first == 0:
                self.erase(self.y, min(self.x, self.cols), self.cols)
            elif first == 1:
                self.erase(self.y, 0, min(self.x + 1, self.cols))
            else:
                self.erase(self.y, 0, self.cols)
        elif final == "n" and first == 6:
            reply(f"\x1b[{self.y + 1};{min(self.x, self.cols - 1) + 1}R")

    def dump(self, path):
        lines = [f"cursor={int(self.cursor_shown)}"] + ["".join(row).rstrip() for row in self.grid]
        with open(path + ".tmp", "w", encoding="utf-8") as f:
            f.write("\n".join(lines) + "\n")
        os.replace(path + ".tmp", path)

    def feed(self, text, reply, dump_path):
        """Handles what it can of text; returns what waits for more bytes."""
        i = 0
        while i < len(text):
            ch = text[i]
            if ch == "\x1b":
                rest = text[i : i + 4096]
                if rest.startswith("\x1b]"):
                    m = OSC.match(rest)
                    if not m:
                        return text[i:]
                    if m.group(1) == "9999;dump":
                        self.dump(dump_path)
                elif rest.startswith("\x1b["):
                    m = CSI.match(rest)
                    if not m:
                        return text[i:]
                    self.csi(m.group(1), m.group(2), m.group(4), reply)
                elif len(rest) < 2:
                    return text[i:]
                else:
                    i += 2
                    continue
                i += m.end()
                continue
            if ch == "\n":
                self.line_feed()
            elif ch == "\r":
                self.x = 0
            elif ch == "\b":
                self.x = max(self.x - 1, 0)
            elif ch >= " " and ch != "\x7f":
                self.put(ch)
            i += 1
        return ""


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    dump_path, command = sys.argv[1], sys.argv[2:]
    pid, fd = pty.fork()
    if pid == 0:
        os.environ.update(TERM="xterm-256color", AURORA_TEST_SCREEN=dump_path)
        os.execvp(command[0], command)
    size = struct.pack("HHHH", 30, 120, 0, 0)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, size)
    screen = Screen(120, 30)
    decoder = codecs.getincrementaldecoder("utf-8")("replace")
    pending = ""
    deadline = time.monotonic() + TIMEOUT_SECONDS
    reply = lambda s: os.write(fd, s.encode())
    while time.monotonic() < deadline:
        if select.select([fd], [], [], 0.1)[0]:
            try:
                data = os.read(fd, 65536)
            except OSError:
                break
            if not data:
                break
            rows, cols = struct.unpack("HHHH", fcntl.ioctl(fd, termios.TIOCGWINSZ, size))[:2]
            if (cols, rows) != (screen.cols, screen.rows):
                screen.resize(cols, rows)
            pending = screen.feed(pending + decoder.decode(data), reply, dump_path)
    else:
        os.kill(pid, 9)
        print("Console test timed out.", file=sys.stderr)
    _, status = os.waitpid(pid, 0)
    sys.exit(os.waitstatus_to_exitcode(status))


if __name__ == "__main__":
    main()
