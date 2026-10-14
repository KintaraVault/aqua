#!/usr/bin/env python3
"""Accessibility-only test fixture; all input is synthetic and publicly visible."""
import os
import sys
import time
import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, GLib

mode = sys.argv[1] if len(sys.argv) > 1 else "gtk"
window = Gtk.Window(title="Aqua accessibility test")
window.set_default_size(760, 400)
window.connect("destroy", Gtk.main_quit)
box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=22)
box.set_border_width(48)
window.add(box)
box.pack_start(Gtk.Label(label="Native suggestions — without a browser extension"), False, False, 0)
entry = Gtk.Entry()
entry.set_text("при" if mode == "gtk-unicode" else "hel")
box.pack_start(entry, False, False, 0)
other = Gtk.Entry()
other.set_placeholder_text("Cancellation target")
box.pack_start(other, False, False, 0)
box.pack_start(Gtk.Label(label="AT-SPI field geometry · paced Unicode text · no clipboard"), False, False, 0)
previous = entry.get_text()
times = []

def changed(widget):
    global previous
    current = widget.get_text()
    delta = len(current) - len(previous)
    if delta > 0:
        times.append(time.monotonic_ns())
        print(f"INSERT chars={delta} ns={times[-1]}", flush=True)
        if mode == "gtk-cancel":
            other.grab_focus()
            print("INPUT_DISABLED_AFTER_FIRST", flush=True)
    previous = current
    print("TEXT=" + current, flush=True)

entry.connect("changed", changed)
window.show_all()

def ready():
    entry.grab_focus()
    entry.set_position(-1)
    print("INPUT_ENABLED", flush=True)
    return False
GLib.timeout_add(500, ready)
Gtk.main()
