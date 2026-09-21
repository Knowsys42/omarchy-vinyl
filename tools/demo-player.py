#!/usr/bin/env python3
"""A fake MPRIS player for testing Vinyl without a real music app.

    tools/demo-player.py [--art cover.png] [--title T] [--artist A] [--album B]

Publishes org.mpris.MediaPlayer2.vinyldemo on the session bus, starts Playing,
and honours PlayPause/Play/Pause/Next/Previous. Ctrl-C to stop.
"""
import argparse, os, signal, time
import gi
gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib

XML = """
<node>
  <interface name="org.mpris.MediaPlayer2">
    <method name="Raise"/><method name="Quit"/>
    <property name="CanQuit" type="b" access="read"/>
    <property name="CanRaise" type="b" access="read"/>
    <property name="HasTrackList" type="b" access="read"/>
    <property name="Identity" type="s" access="read"/>
    <property name="SupportedUriSchemes" type="as" access="read"/>
    <property name="SupportedMimeTypes" type="as" access="read"/>
  </interface>
  <interface name="org.mpris.MediaPlayer2.Player">
    <method name="Next"/><method name="Previous"/><method name="Pause"/>
    <method name="PlayPause"/><method name="Stop"/><method name="Play"/>
    <method name="Seek"><arg name="Offset" type="x" direction="in"/></method>
    <method name="SetPosition"><arg name="TrackId" type="o" direction="in"/><arg name="Position" type="x" direction="in"/></method>
    <method name="OpenUri"><arg name="Uri" type="s" direction="in"/></method>
    <signal name="Seeked"><arg name="Position" type="x"/></signal>
    <property name="PlaybackStatus" type="s" access="read"/>
    <property name="Rate" type="d" access="readwrite"/>
    <property name="Metadata" type="a{sv}" access="read"/>
    <property name="Volume" type="d" access="readwrite"/>
    <property name="Position" type="x" access="read"/>
    <property name="MinimumRate" type="d" access="read"/>
    <property name="MaximumRate" type="d" access="read"/>
    <property name="CanGoNext" type="b" access="read"/>
    <property name="CanGoPrevious" type="b" access="read"/>
    <property name="CanPlay" type="b" access="read"/>
    <property name="CanPause" type="b" access="read"/>
    <property name="CanSeek" type="b" access="read"/>
    <property name="CanControl" type="b" access="read"/>
  </interface>
</node>
"""

class Player:
    def __init__(self, a):
        self.a = a
        self.status = "Playing"
        self.pos_at = time.monotonic()
        self.pos_us = a.start * 1_000_000
        self.length_us = a.length * 1_000_000
        self.conn = None

    def position(self):
        p = self.pos_us
        if self.status == "Playing":
            p += int((time.monotonic() - self.pos_at) * 1e6)
        return min(p, self.length_us)

    def metadata(self):
        m = {
            "mpris:trackid": GLib.Variant("o", "/org/mpris/MediaPlayer2/vinyldemo/1"),
            "mpris:length": GLib.Variant("x", self.length_us),
            "xesam:title": GLib.Variant("s", self.a.title),
            "xesam:artist": GLib.Variant("as", [self.a.artist]),
            "xesam:album": GLib.Variant("s", self.a.album),
        }
        if self.a.art:
            m["mpris:artUrl"] = GLib.Variant("s", "file://" + os.path.abspath(self.a.art))
        return GLib.Variant("a{sv}", m)

    def get_prop(self, conn, sender, path, iface, name):
        v = {
            "CanQuit": ("b", True), "CanRaise": ("b", False), "HasTrackList": ("b", False),
            "Identity": ("s", self.a.identity), "SupportedUriSchemes": ("as", []), "SupportedMimeTypes": ("as", []),
            "PlaybackStatus": ("s", self.status), "Rate": ("d", 1.0), "Volume": ("d", 1.0),
            "Position": ("x", self.position()), "MinimumRate": ("d", 1.0), "MaximumRate": ("d", 1.0),
            "CanGoNext": ("b", True), "CanGoPrevious": ("b", True), "CanPlay": ("b", True),
            "CanPause": ("b", True), "CanSeek": ("b", True), "CanControl": ("b", True),
        }
        if name == "Metadata":
            return self.metadata()
        t, val = v[name]
        return GLib.Variant(t, val)

    def set_prop(self, *args):
        return True

    def call(self, conn, sender, path, iface, method, params, inv):
        if method in ("PlayPause", "Play", "Pause", "Stop"):
            playing = self.status == "Playing"
            want = {"PlayPause": not playing, "Play": True, "Pause": False, "Stop": False}[method]
            self.pos_us = self.position()
            self.pos_at = time.monotonic()
            self.status = "Playing" if want else "Paused"
            self.changed({"PlaybackStatus": GLib.Variant("s", self.status)})
        elif method in ("Seek", "SetPosition"):
            offset = params[0] if method == "Seek" else None
            want = self.position() + offset if offset is not None else params[1]
            self.pos_us = max(0, min(want, self.length_us))
            self.pos_at = time.monotonic()
            self.conn.emit_signal(None, "/org/mpris/MediaPlayer2", "org.mpris.MediaPlayer2.Player",
                                  "Seeked", GLib.Variant("(x)", (self.pos_us,)))
        elif method in ("Next", "Previous"):
            self.pos_us, self.pos_at = 0, time.monotonic()
            self.changed({"Metadata": self.metadata()})
        elif method == "Quit":
            loop.quit()
        inv.return_value(None)

    def changed(self, props):
        self.conn.emit_signal(None, "/org/mpris/MediaPlayer2", "org.freedesktop.DBus.Properties",
                              "PropertiesChanged",
                              GLib.Variant("(sa{sv}as)", ("org.mpris.MediaPlayer2.Player", props, [])))

ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
ap.add_argument("--art"); ap.add_argument("--title", default="Midnight Pressing")
ap.add_argument("--artist", default="The Layer Shells"); ap.add_argument("--album", default="Side A")
ap.add_argument("--identity", default="MPRIS Demo"); ap.add_argument("--length", type=int, default=214)
ap.add_argument("--start", type=int, default=0); ap.add_argument("--paused", action="store_true")
a = ap.parse_args()
player = Player(a)
if a.paused:
    player.status = "Paused"
loop = GLib.MainLoop()
info = Gio.DBusNodeInfo.new_for_xml(XML)

def on_bus(conn, name):
    player.conn = conn
    for iface in info.interfaces:
        conn.register_object("/org/mpris/MediaPlayer2", iface, player.call, player.get_prop, player.set_prop)

Gio.bus_own_name(Gio.BusType.SESSION, "org.mpris.MediaPlayer2.vinyldemo", Gio.BusNameOwnerFlags.NONE, on_bus, None, lambda *_: loop.quit())
signal.signal(signal.SIGINT, lambda *_: loop.quit())
signal.signal(signal.SIGTERM, lambda *_: loop.quit())
loop.run()
