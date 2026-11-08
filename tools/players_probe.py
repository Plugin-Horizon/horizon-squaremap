#!/usr/bin/env python3
"""Joins a test server with a bare protocol client and reads the map's players.json while
the player is visible, invisible (effect) and a spectator.

Usage: players_probe.py <game port> <web port> <screen session> <path to suggest_probe.py>
(suggest_probe.py: a bare offline-mode protocol client with a `Conn` class.)
"""
import importlib.util
import json
import struct
import subprocess
import sys
import time
import urllib.request

port, web, session, probe = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3], sys.argv[4]
spec = importlib.util.spec_from_file_location("sp", probe)
sp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sp)
c = sp.Conn(port, "MapProbe")
pos, last = None, 0.0


def console(cmd):
    subprocess.run(["screen", "-S", session, "-X", "stuff", cmd + "\r"], check=True)


def pump(seconds):
    global pos, last
    end = time.time() + seconds
    while time.time() < end:
        try:
            pid, d = c.recv()
        except Exception:
            pid = None
        if pid == 45:  # keep-alive
            c.send(28, d)
        elif pid == 73:  # player position: accept the teleport
            tid, i = sp.parse_varint(d)
            pos = struct.unpack(">ddd", d[i:i + 24])
            c.send(0, sp.varint(tid) + struct.pack(">dddff", *pos, 0.0, 0.0))
            c.send(44)
        if pos and time.time() - last > 0.5 and pid is not None:
            c.send(31, struct.pack(">dddffB", *pos, 0.0, 0.0, 1))
            c.send(13)
            last = time.time()


def players():
    with urllib.request.urlopen(f"http://127.0.0.1:{web}/tiles/players.json", timeout=5) as r:
        return json.load(r)


pump(4)
print("visible:     ", players())
console("effect give MapProbe minecraft:invisibility 60 0 true")
pump(3)
print("invisible:   ", players())
console("effect clear MapProbe")
console("gamemode spectator MapProbe")
pump(3)
print("spectator:   ", players())
console("gamemode survival MapProbe")
pump(3)
print("survival:    ", players())
