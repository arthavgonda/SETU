import QtQuick
import Setu.Theme

Canvas {
    id: eyes
    property string mood: "idle"
    property real level: 0
    property real open: 1
    property real look: 0
    property real lift: 0
    property real bob: 0
    property real eyeHeight: Math.round(height * 0.62)
    property color ink: Theme.notchInk
    readonly property bool moving: visible && !Motion.reduced
    readonly property var key: [mood, open, look, lift, bob, level, width, height, ink]
    implicitWidth: Theme.px(40)
    implicitHeight: Theme.px(20)
    renderStrategy: Canvas.Cooperative
    renderTarget: Canvas.Image
    Accessible.ignored: true
    onKeyChanged: requestPaint()

    function blinkNow() {
        if (moving && mood !== "sleepy" && mood !== "happy")
            blink.restart()
    }

    SequentialAnimation {
        id: blink
        NumberAnimation { target: eyes; property: "open"; to: 0.08; duration: 70; easing.type: Easing.InQuad }
        NumberAnimation { target: eyes; property: "open"; to: 1; duration: 130; easing.type: Easing.OutBack; easing.overshoot: 2.2 }
    }

    Timer {
        interval: 2600 + Math.random() * 3200
        repeat: true
        running: eyes.moving && eyes.mood !== "sleepy"
        onTriggered: {
            interval = 2400 + Math.random() * 3600
            eyes.blinkNow()
            if (eyes.mood === "idle" && Math.random() < 0.35)
                glance.restart()
        }
    }

    SequentialAnimation {
        id: glance
        NumberAnimation { target: eyes; property: "look"; to: Math.random() < 0.5 ? -1 : 1; duration: 220; easing.type: Easing.OutCubic }
        PauseAnimation { duration: 700 }
        NumberAnimation { target: eyes; property: "look"; to: 0; duration: 260; easing.type: Easing.InOutCubic }
    }

    SequentialAnimation {
        running: eyes.moving && eyes.mood === "think"
        loops: Animation.Infinite
        onRunningChanged: if (!running) { eyes.look = 0; eyes.lift = 0 }
        ParallelAnimation {
            NumberAnimation { target: eyes; property: "look"; to: -1; duration: 380; easing.type: Easing.InOutSine }
            NumberAnimation { target: eyes; property: "lift"; to: -1; duration: 380; easing.type: Easing.InOutSine }
        }
        PauseAnimation { duration: 260 }
        NumberAnimation { target: eyes; property: "look"; to: 1; duration: 520; easing.type: Easing.InOutSine }
        PauseAnimation { duration: 260 }
    }

    SequentialAnimation {
        running: eyes.moving && (eyes.mood === "talk" || eyes.mood === "happy")
        loops: Animation.Infinite
        onRunningChanged: if (!running) eyes.bob = 0
        NumberAnimation { target: eyes; property: "bob"; to: -1; duration: 240; easing.type: Easing.InOutSine }
        NumberAnimation { target: eyes; property: "bob"; to: 0.4; duration: 260; easing.type: Easing.InOutSine }
    }

    onMoodChanged: {
        if (mood === "wow" && moving)
            pop.restart()
        else if (mood === "sad" && moving)
            shake.restart()
    }

    SequentialAnimation {
        id: pop
        NumberAnimation { target: eyes; property: "scale"; to: 1.25; duration: 120; easing.type: Easing.OutQuad }
        NumberAnimation { target: eyes; property: "scale"; to: 1; duration: 380; easing.type: Easing.OutBack; easing.overshoot: 3 }
    }

    SequentialAnimation {
        id: shake
        NumberAnimation { target: eyes; property: "look"; to: -1; duration: 60 }
        NumberAnimation { target: eyes; property: "look"; to: 1; duration: 90 }
        NumberAnimation { target: eyes; property: "look"; to: -0.6; duration: 90 }
        NumberAnimation { target: eyes; property: "look"; to: 0; duration: 120; easing.type: Easing.OutBack }
    }

    onPaint: {
        const ctx = getContext("2d")
        ctx.reset()
        const w = width
        const h = height
        if (w <= 0 || h <= 0)
            return
        const ew = Math.max(3, Math.round(h * 0.34))
        const gap = Math.round(w * 0.42)
        const cx = w / 2 + look * w * 0.06
        const cy = h / 2 + lift * h * 0.10 + bob * h * 0.05
        let eh = eyeHeight
        if (mood === "listen")
            eh = eyeHeight * (1.05 + Math.min(0.3, level * 2.4))
        else if (mood === "wow")
            eh = eyeHeight * 1.15
        else if (mood === "focus")
            eh = eyeHeight * 0.78
        eh = Math.max(ew * 0.5, eh * open)
        ctx.fillStyle = ink
        ctx.strokeStyle = ink
        ctx.shadowColor = Qt.rgba(1, 1, 1, 0.55)
        ctx.shadowBlur = Math.round(h * 0.35)
        for (let side = -1; side <= 1; side += 2) {
            const x = cx + side * gap / 2
            ctx.save()
            if (mood === "happy") {
                ctx.lineWidth = Math.max(2, ew * 0.62)
                ctx.lineCap = "round"
                ctx.beginPath()
                ctx.arc(x, cy + ew * 0.55, ew * 0.95, Math.PI * 1.12, Math.PI * 1.88, false)
                ctx.stroke()
            } else if (mood === "sleepy") {
                ctx.lineWidth = Math.max(2, ew * 0.55)
                ctx.lineCap = "round"
                ctx.beginPath()
                ctx.arc(x, cy - ew * 0.5, ew * 0.95, Math.PI * 0.15, Math.PI * 0.85, false)
                ctx.stroke()
            } else {
                ctx.translate(x, cy)
                if (mood === "sad")
                    ctx.rotate(side * -0.32)
                const r = Math.min(ew, eh) / 2
                ctx.beginPath()
                ctx.roundedRect(-ew / 2, -eh / 2, ew, eh, r, r)
                ctx.fill()
            }
            ctx.restore()
        }
    }
}
