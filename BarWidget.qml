import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Services.Mpris
import qs.Commons
import qs.Ui

// A little record in the bar. It turns while any MPRIS player is playing.
// Left click opens or closes the desktop widget, right click opens the
// full-screen view, middle click steps to the next pressing.
BarWidget {
  id: root
  moduleName: "io.github.knowsys42.vinyl"

  readonly property string pluginDir: Quickshell.env("HOME") + "/.config/omarchy/plugins/io.github.knowsys42.vinyl"
  readonly property string ctl: pluginDir + "/bin/vinyl-ctl"
  readonly property string extraArgs: root.setting("args", "")
  readonly property bool showTitle: root.setting("showTitle", false)

  readonly property var players: Mpris.players ? Mpris.players.values : []
  readonly property var playing: {
    for (var i = 0; i < players.length; i++) {
      var p = players[i]
      if (p && p.playbackState === MprisPlaybackState.Playing) return p
    }
    return null
  }
  readonly property bool isPlaying: playing !== null
  readonly property string title: playing ? (playing.trackTitle || "") : ""

  function run(action) {
    Quickshell.execDetached({
      command: [root.ctl, action],
      environment: { VINYL_ARGS: root.extraArgs }
    })
  }

  implicitWidth: root.vertical ? root.barSize : row.implicitWidth + 14
  implicitHeight: root.vertical ? row.implicitHeight + 10 : root.barSize

  Rectangle {
    anchors.fill: parent
    radius: 4
    color: mouse.containsMouse ? Qt.rgba(1, 1, 1, 0.08) : "transparent"
  }

  Row {
    id: row
    anchors.centerIn: parent
    spacing: 6

    Canvas {
      id: disc
      width: 16
      height: 16
      anchors.verticalCenter: parent.verticalCenter
      rotation: 0
      readonly property color ink: root.bar ? root.bar.foreground : "#ffffff"
      onInkChanged: requestPaint()
      onPaint: {
        var ctx = getContext("2d")
        var c = width / 2
        ctx.reset()
        ctx.clearRect(0, 0, width, height)
        ctx.globalAlpha = root.isPlaying ? 1.0 : 0.55
        ctx.fillStyle = ink
        ctx.beginPath(); ctx.arc(c, c, c, 0, Math.PI * 2); ctx.fill()
        ctx.globalAlpha = root.isPlaying ? 0.35 : 0.2
        ctx.strokeStyle = root.bar ? root.bar.background : "#000000"
        ctx.lineWidth = 0.8
        for (var r = 4.2; r < c - 0.8; r += 1.4) {
          ctx.beginPath(); ctx.arc(c, c, r, 0, Math.PI * 2); ctx.stroke()
        }
        ctx.globalAlpha = 1.0
        ctx.fillStyle = root.bar ? root.bar.background : "#000000"
        ctx.beginPath(); ctx.arc(c, c, 3.2, 0, Math.PI * 2); ctx.fill()
        ctx.fillStyle = ink
        ctx.beginPath(); ctx.arc(c + 1.6, c, 1.0, 0, Math.PI * 2); ctx.fill()
      }

      RotationAnimation on rotation {
        running: root.isPlaying
        loops: Animation.Infinite
        from: 0
        to: 360
        duration: 1800
      }
    }

    Text {
      visible: root.showTitle && !root.vertical && root.title !== ""
      anchors.verticalCenter: parent.verticalCenter
      text: root.title
      color: root.bar ? root.bar.foreground : "#ffffff"
      font.pixelSize: Style.font ? Style.font.sizeSmall || 12 : 12
      elide: Text.ElideRight
      width: Math.min(implicitWidth, 160)
    }
  }

  Connections {
    target: root
    function onIsPlayingChanged() { disc.requestPaint() }
  }

  MouseArea {
    id: mouse
    anchors.fill: parent
    hoverEnabled: true
    acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton
    onClicked: function (event) {
      if (event.button === Qt.LeftButton) root.run("toggle")
      else if (event.button === Qt.RightButton) root.run("fullscreen")
      else root.run("style")
    }
  }
}
