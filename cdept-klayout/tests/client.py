# cdept-klayout tests: a minimal CDePT side of the protocol (plain python3).
# SPDX-License-Identifier: GPL-3.0-or-later
"""Starts the helper the way CDePT does and talks to it.

KLayout to use: $CDEPT_TEST_KLAYOUT (a `klayout` binary), else
../.bin/klayout. With CDEPT_TEST_DOCKER=<image> the helper runs in that image
instead (the test folders are bind-mounted at the same paths).
"""

import json
import mmap
import os
import select
import socket
import subprocess
import time

HERE = os.path.dirname(os.path.abspath(__file__))
HELPER = os.path.join(os.path.dirname(HERE), "helper", "cdept_klayout", "main.py")
DEFAULT_KLAYOUT = os.path.join(os.path.dirname(HERE), ".bin", "klayout")


def helper_command(socket_path, frame_dir=None, display=False):
    """argv (and environment additions) that start the helper."""
    args = ["-z", "-nc", "-e", "-r", HELPER, "-rd", "connect=" + socket_path]
    if frame_dir:
        args += ["-rd", "frame_dir=" + frame_dir]
    if display:
        args += ["-rd", "display=1"]
    image = os.environ.get("CDEPT_TEST_DOCKER")
    if image:
        mounts = []
        for folder in sorted({os.path.dirname(socket_path), os.path.dirname(os.path.dirname(HELPER)),
                              frame_dir or os.path.dirname(socket_path)}):
            mounts += ["-v", "%s:%s" % (folder, folder)]
        return ["docker", "run", "--rm", "-i", "--user", "%d:%d" % (os.getuid(), os.getgid()),
                "-e", "QT_QPA_PLATFORM=offscreen", "-e", "HOME=/tmp"] + mounts + \
            ["--entrypoint", "klayout", image] + args
    klayout = os.environ.get("CDEPT_TEST_KLAYOUT") or DEFAULT_KLAYOUT
    return [klayout] + args


class Helper:
    """One running helper."""

    def __init__(self, folder, display=False, timeout=60):
        self.folder = folder
        self.socket_path = os.path.join(folder, "helper.sock")
        listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        listener.bind(self.socket_path)
        os.chmod(self.socket_path, 0o777)
        listener.listen(1)
        env = dict(os.environ, QT_QPA_PLATFORM="offscreen",
                   KLAYOUT_HOME=os.path.join(folder, "klayout-home"))
        started = time.monotonic()
        self.process = subprocess.Popen(
            helper_command(self.socket_path, folder if display else None, display), env=env,
            stdin=subprocess.DEVNULL)
        listener.settimeout(timeout)
        self.sock, _ = listener.accept()
        listener.close()
        self.buffer = b""
        self.next_id = 1
        self.events = []
        self.frames = []
        self.damage = []
        self.ready = self.wait(lambda m: m["type"] in ("ready", "failed"), timeout)
        self.startup = time.monotonic() - started
        self.maps = {}

    def send(self, message):
        self.sock.sendall((json.dumps(message) + "\n").encode())

    def _read(self, timeout):
        ready, _, _ = select.select([self.sock], [], [], timeout)
        if not ready:
            return []
        chunk = self.sock.recv(1 << 20)
        if not chunk:
            raise EOFError("helper closed the connection")
        self.buffer += chunk
        out = []
        while b"\n" in self.buffer:
            line, self.buffer = self.buffer.split(b"\n", 1)
            out.append(json.loads(line))
        return out

    def _keep(self, m):
        if m["type"] == "event":
            self.events.append(m["event"])
        elif m["type"] == "frame":
            self.frames.append(m)
        elif m["type"] == "damage":
            self.damage.append(m)

    def wait(self, test, timeout=30):
        """Read messages until one passes `test`; others are kept."""
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            for m in self._read(0.05):
                self._keep(m)
                if test(m):
                    return m
        raise TimeoutError("no matching message within %ss" % timeout)

    def call(self, method, timeout=60, **params):
        """Call and wait for the reply: the value, or raise with the error."""
        id = self.next_id
        self.next_id += 1
        self.send({"type": "call", "id": id, "method": method, "params": params})
        reply = self.wait(lambda m: m["type"] == "reply" and m["id"] == id, timeout)
        if not reply["ok"]:
            raise CallError(reply.get("error", ""))
        return reply.get("value")

    def input(self, **event):
        event.setdefault("modifiers", {})
        self.send({"type": "input", "input": event})

    def click(self, x, y, button="left"):
        self.input(kind="move", x=x, y=y, buttons=[])
        self.input(kind="press", x=x, y=y, button=button)
        self.input(kind="release", x=x, y=y, button=button)

    def settle(self, quiet=0.3):
        """Read until nothing arrives for `quiet` seconds."""
        last = time.monotonic()
        while time.monotonic() - last < quiet:
            messages = self._read(0.05)
            for m in messages:
                self._keep(m)
            if messages:
                last = time.monotonic()

    def last_event(self, kind):
        for e in reversed(self.events):
            if e["kind"] == kind:
                return e
        return None

    def pixels(self, frame):
        """The bytes of a frame file (B G R A)."""
        with open(frame["path"], "rb") as f:
            m = mmap.mmap(f.fileno(), frame["stride"] * frame["height"], access=mmap.ACCESS_READ)
            data = bytes(m)
            m.close()
        return data

    def close(self):
        try:
            self.send({"type": "shutdown", "id": 0})
            self.wait(lambda m: m["type"] == "reply" and m["id"] == 0, 10)
        except Exception:
            pass
        try:
            self.process.wait(10)
        except subprocess.TimeoutExpired:
            self.process.kill()
        self.sock.close()


class CallError(Exception):
    pass
