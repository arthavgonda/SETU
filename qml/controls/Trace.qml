import QtQuick
import Setu.Core
import Setu.Theme

Canvas {
    id: trace
    property bool live: visible
    property color lineColor: active ? Theme.accent : Theme.borderStrong
    property bool active: level > 0.004
    property real level: 0
    property int points: 96
    property var history: []
    property bool dirty: false
    implicitHeight: Theme.px(36)
    implicitWidth: 400
    renderStrategy: Canvas.Cooperative
    renderTarget: Canvas.Image
    Accessible.role: Accessible.Indicator
    Accessible.name: qsTr("Microphone level")
    Accessible.description: level > 0.05 ? qsTr("Sound detected") : qsTr("Quiet")

    Subscription {
        topics: ["audio.level"]
        active: trace.live
        onReceived: (topic, data) => {
            trace.level = Math.min(1, Number(data.rms) * 4)
            let h = trace.history
            h.push(trace.level)
            if (h.length > trace.points)
                h.shift()
            trace.history = h
            trace.dirty = true
        }
    }
    onLiveChanged: if (!live) {
        level = 0
        history = []
        requestPaint()
    }
    Timer {
        interval: 33
        repeat: true
        running: trace.live && trace.dirty
        onTriggered: {
            trace.dirty = false
            trace.requestPaint()
        }
    }
    onPaint: {
        const ctx = getContext("2d")
        ctx.reset()
        const mid = height / 2
        ctx.lineWidth = 1.5
        ctx.strokeStyle = lineColor
        ctx.lineCap = "round"
        ctx.lineJoin = "round"
        if (Motion.reduced) {
            ctx.fillStyle = Theme.bgSunken
            ctx.fillRect(0, mid - 2, width, 4)
            ctx.fillStyle = lineColor
            ctx.fillRect(0, mid - 2, width * level, 4)
            return
        }
        ctx.beginPath()
        const n = history.length
        if (n < 2) {
            ctx.moveTo(0, mid)
            ctx.lineTo(width, mid)
        } else {
            const step = width / (points - 1)
            const offset = (points - n) * step
            ctx.moveTo(0, mid)
            ctx.lineTo(offset, mid)
            for (let i = 0; i < n; ++i) {
                const amp = history[i] * (height / 2 - 2)
                const y = mid + (i % 2 === 0 ? -amp : amp)
                ctx.lineTo(offset + i * step, y)
            }
        }
        ctx.stroke()
    }
}
