import QtQuick
import Setu.Theme
import Ledger

Canvas {
    id: ring
    property real progress: 0
    property color stroke: Theme.notchTimer
    property color track: Qt.rgba(1, 1, 1, 0.16)
    property real thickness: Math.max(2, width * 0.12)
    property string glyph: ""
    readonly property var key: [progress, stroke, track, width, height, thickness]
    implicitWidth: Theme.px(18)
    implicitHeight: implicitWidth
    renderStrategy: Canvas.Cooperative
    renderTarget: Canvas.Image
    Accessible.ignored: true
    onKeyChanged: requestPaint()
    onPaint: {
        const ctx = getContext("2d")
        ctx.reset()
        const r = Math.min(width, height) / 2 - thickness / 2
        const cx = width / 2
        const cy = height / 2
        ctx.lineWidth = thickness
        ctx.lineCap = "round"
        ctx.strokeStyle = track
        ctx.beginPath()
        ctx.arc(cx, cy, r, 0, Math.PI * 2, false)
        ctx.stroke()
        const p = Math.max(0, Math.min(1, progress))
        if (p > 0.001) {
            ctx.strokeStyle = stroke
            ctx.beginPath()
            ctx.arc(cx, cy, r, -Math.PI / 2, -Math.PI / 2 + p * Math.PI * 2, false)
            ctx.stroke()
        }
    }
    Icon {
        anchors.centerIn: parent
        visible: ring.glyph.length > 0
        name: ring.glyph
        color: ring.stroke
        size: Math.round(ring.width * 0.44)
    }
}
