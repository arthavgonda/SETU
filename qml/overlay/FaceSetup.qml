import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Setu.ShellHost
import Ledger

Window {
    id: win
    property string mode: "enrol"
    property string frame: ""
    property string message: ""
    property var steps: ({})
    readonly property bool enrol: mode === "enrol"
    readonly property var sides: ["up", "right", "down", "left"]
    readonly property bool finished: enrol && sides.every(s => steps[s] === true) && steps.center === true
    readonly property bool calm: Motion.reduced
    readonly property int disc: Theme.px(enrol ? 232 : 168)
    visible: true
    color: "transparent"
    width: card.width + 16
    height: card.height + 16
    title: enrol ? qsTr("Set up face unlock") : qsTr("Checking it's you")
    Component.onCompleted: {
        OverlayHost.prepare(win, false)
        OverlayHost.place(win, width, height, 24 + Theme.px(220))
        OverlayHost.setInputRegion(win, [Qt.rect(0, 0, width, height)])
        Announcer.announce(card, win.title)
    }
    onMessageChanged: if (message.length > 0) Announcer.announce(card, message)

    Rectangle {
        id: card
        x: 8
        y: 8
        width: Theme.px(enrol ? 340 : 260)
        height: col.implicitHeight + Theme.px(40)
        radius: Theme.px(28)
        color: Theme.notchBg
        border.width: 1
        border.color: Theme.notchRaised
        Accessible.role: Accessible.Pane
        Accessible.name: win.title
        scale: shown ? 1 : 0.92
        opacity: shown ? 1 : 0
        property bool shown: false
        Component.onCompleted: shown = true
        Behavior on scale {
            NumberAnimation { duration: win.calm ? 0 : 320; easing.type: Easing.OutBack }
        }
        Behavior on opacity {
            NumberAnimation { duration: win.calm ? 0 : 180 }
        }

        ColumnLayout {
            id: col
            anchors.top: parent.top
            anchors.topMargin: Theme.px(22)
            anchors.horizontalCenter: parent.horizontalCenter
            width: parent.width - Theme.px(40)
            spacing: Theme.px(16)

            Text {
                Layout.fillWidth: true
                horizontalAlignment: Text.AlignHCenter
                text: win.title
                color: Theme.notchInk
                font.family: Type.sans
                font.pixelSize: Theme.px(17)
                font.weight: Font.DemiBold
            }

            Item {
                id: stage
                Layout.alignment: Qt.AlignHCenter
                Layout.preferredWidth: win.disc + Theme.px(28)
                Layout.preferredHeight: win.disc + Theme.px(28)

                Item {
                    id: picture
                    anchors.centerIn: parent
                    width: win.disc
                    height: win.disc
                    Rectangle {
                        anchors.fill: parent
                        radius: width / 2
                        color: Theme.notchCard
                    }
                    Image {
                        anchors.fill: parent
                        source: win.frame.length > 0 ? "data:image/jpeg;base64," + win.frame : ""
                        fillMode: Image.PreserveAspectCrop
                        cache: false
                        asynchronous: false
                        smooth: true
                        Accessible.ignored: true
                    }
                    Canvas {
                        id: mask
                        anchors.fill: parent
                        onWidthChanged: requestPaint()
                        onPaint: {
                            const c = getContext("2d")
                            c.reset()
                            c.fillStyle = Theme.notchBg
                            c.fillRect(0, 0, width, height)
                            c.globalCompositeOperation = "destination-out"
                            c.beginPath()
                            c.arc(width / 2, height / 2, width / 2 - 1, 0, Math.PI * 2)
                            c.fill()
                        }
                    }
                    Rectangle {
                        anchors.centerIn: parent
                        width: Theme.px(64)
                        height: width
                        radius: width / 2
                        color: Theme.notchDone
                        scale: win.finished ? 1 : 0
                        opacity: win.finished ? 1 : 0
                        Behavior on scale {
                            NumberAnimation { duration: win.calm ? 0 : 420; easing.type: Easing.OutBack; easing.overshoot: 2.2 }
                        }
                        Behavior on opacity {
                            NumberAnimation { duration: win.calm ? 0 : 160 }
                        }
                        Text {
                            anchors.centerIn: parent
                            text: "✓"
                            color: Theme.notchBg
                            font.pixelSize: Theme.px(36)
                            font.weight: Font.Bold
                        }
                    }
                }

                Repeater {
                    model: win.enrol ? win.sides : []
                    delegate: Canvas {
                        id: arc
                        required property string modelData
                        required property int index
                        property real fill: win.steps[modelData] === true ? 1 : 0
                        anchors.fill: parent
                        Behavior on fill {
                            NumberAnimation { duration: win.calm ? 0 : 520; easing.type: Easing.OutCubic }
                        }
                        onFillChanged: requestPaint()
                        onWidthChanged: requestPaint()
                        onPaint: {
                            const c = getContext("2d")
                            c.reset()
                            const r = width / 2 - Theme.px(6)
                            const mid = -Math.PI / 2 + index * Math.PI / 2
                            const half = Math.PI / 4 - 0.12
                            c.lineCap = "round"
                            c.lineWidth = Theme.px(6)
                            c.strokeStyle = Theme.notchRaised
                            c.beginPath()
                            c.arc(width / 2, height / 2, r, mid - half, mid + half)
                            c.stroke()
                            if (fill <= 0)
                                return
                            const g = c.createLinearGradient(0, 0, width, height)
                            g.addColorStop(0, Theme.notchListen)
                            g.addColorStop(0.45, Theme.notchListenDeep)
                            g.addColorStop(0.75, Theme.notchSpeak)
                            g.addColorStop(1, Theme.notchSpeakDeep)
                            c.strokeStyle = g
                            c.beginPath()
                            c.arc(width / 2, height / 2, r, mid - half, mid - half + 2 * half * fill)
                            c.stroke()
                        }
                    }
                }

                Canvas {
                    id: scan
                    visible: !win.enrol
                    anchors.fill: parent
                    property real turn: 0
                    NumberAnimation on turn {
                        running: scan.visible && !win.calm
                        from: 0
                        to: Math.PI * 2
                        duration: 1400
                        loops: Animation.Infinite
                    }
                    onTurnChanged: requestPaint()
                    onPaint: {
                        const c = getContext("2d")
                        c.reset()
                        const r = width / 2 - Theme.px(6)
                        c.lineCap = "round"
                        c.lineWidth = Theme.px(5)
                        c.strokeStyle = Theme.notchRaised
                        c.beginPath()
                        c.arc(width / 2, height / 2, r, 0, Math.PI * 2)
                        c.stroke()
                        const g = c.createLinearGradient(0, 0, width, height)
                        g.addColorStop(0, Theme.notchListenDeep)
                        g.addColorStop(1, Theme.notchSpeak)
                        c.strokeStyle = g
                        c.beginPath()
                        c.arc(width / 2, height / 2, r, turn, turn + Math.PI * 0.7)
                        c.stroke()
                    }
                }
            }

            Text {
                Layout.fillWidth: true
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                text: win.message.length > 0 ? win.message : (win.enrol ? qsTr("Look at the camera, then turn your head slowly.") : qsTr("Look at the camera."))
                color: Theme.notchMuted
                font.family: Type.sans
                font.pixelSize: Theme.px(14)
            }

            Text {
                visible: win.enrol
                Layout.fillWidth: true
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                text: qsTr("Only face measurements are kept, never pictures.")
                color: Theme.notchFaint
                font.family: Type.sans
                font.pixelSize: Theme.px(12)
            }

            Rectangle {
                id: cancel
                Layout.alignment: Qt.AlignHCenter
                Layout.preferredWidth: Theme.px(120)
                Layout.preferredHeight: Theme.px(36)
                radius: height / 2
                color: tap.pressed ? Theme.notchMuted : Theme.notchInk
                Accessible.role: Accessible.Button
                Accessible.name: qsTr("Cancel")
                Accessible.onPressAction: Daemon.call("face.cancel", {})
                Text {
                    anchors.centerIn: parent
                    text: qsTr("Cancel")
                    color: Theme.notchBg
                    font.family: Type.sans
                    font.pixelSize: Theme.px(14)
                    font.weight: Font.DemiBold
                }
                TapHandler {
                    id: tap
                    onTapped: Daemon.call("face.cancel", {})
                }
                HoverHandler {
                    cursorShape: Qt.PointingHandCursor
                }
            }
        }
    }
}
