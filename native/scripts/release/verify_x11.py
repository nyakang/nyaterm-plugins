"""Exercise the actual unsigned Windows package before publication."""
import argparse
import json
from pathlib import Path
import socket
import struct
import subprocess
import threading
import queue
import time


def send(process, value):
    raw = json.dumps(value).encode()
    process.stdin.write(struct.pack("<I", len(raw)) + raw)
    process.stdin.flush()


def receive(process):
    replies = queue.Queue(maxsize=1)
    def read():
        try:
            prefix = process.stdout.read(4)
            size, = struct.unpack("<I", prefix)
            if not 0 < size <= 4096:
                raise ValueError("invalid control size")
            replies.put(json.loads(process.stdout.read(size)))
        except Exception:
            replies.put(None)
    threading.Thread(target=read, daemon=True).start()
    return replies.get(timeout=20)


def authenticate(port, cookie):
    with socket.create_connection(("127.0.0.1", port), timeout=2) as connection:
        connection.settimeout(2)
        packet = struct.pack("<BBHHHHH", ord("l"), 0, 11, 0, 18 if cookie else 0, len(cookie), 0)
        connection.sendall(packet + (b"MIT-MAGIC-COOKIE-1\0\0" if cookie else b"") + cookie)
        return connection.recv(1) == b"\1"


def closed(port):
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=.2): return False
    except OSError:
        return True


def verify(root):
    processes = []
    try:
        ports, cookies = [], []
        for _ in range(2):
            process = subprocess.Popen([str(root / "nyaterm-x11-helper.exe")], cwd=root,
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
            processes.append(process)
            send(process, {"Start": {"version": 1, "clipboard": False}})
            event = receive(process)
            if not event or "Ready" not in event: raise RuntimeError("X11 helper did not become ready")
            ready = event["Ready"]
            port = 6000 + ready["display"]
            cookie = bytes.fromhex(ready["cookie"])
            ports.append(port)
            cookies.append(cookie)
            assert authenticate(port, cookie)
            assert not authenticate(port, b"")
            assert not authenticate(port, bytes(byte ^ 255 for byte in cookie))
            # Inspect actual listeners rather than inferring binding from our destination.
            script = f"Get-NetTCPConnection -State Listen -LocalPort {port} | Select-Object -ExpandProperty LocalAddress"
            addresses = subprocess.check_output(["powershell", "-NoProfile", "-Command", script], text=True).split()
            assert addresses and set(addresses) == {"127.0.0.1"}, "non-loopback X11 listener"
        assert ports[0] != ports[1] and cookies[0] != cookies[1]
        assert not authenticate(ports[0], cookies[1])
        send(processes[0], "Stop")
        processes[0].wait(timeout=5)
        assert closed(ports[0]) and authenticate(ports[1], cookies[1])
        # Forced helper death must close its private job and reap VcXsrv.
        processes[1].kill()
        processes[1].wait(timeout=5)
        deadline = time.monotonic() + 5
        while not closed(ports[1]) and time.monotonic() < deadline: time.sleep(.05)
        assert closed(ports[1]), "orphan X11 server"
    finally:
        for process in processes:
            if process.poll() is None: process.kill()
            process.wait(timeout=5)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=Path)
    args = parser.parse_args()
    verify(args.root.resolve())
