import QtQuick
import QtQuick.Layouts
import QtQuick.Window
import Setu.Core
import Setu.Theme
import Ledger

Window {
    id: win
    property bool busy: false
    property string error: ""
    property real travel: Motion.reduced ? 0.55 : 0
    property real folderBump: 1
    signal dismissed

    width: Theme.px(560)
    height: column.implicitHeight + Theme.s6 * 2
    minimumWidth: Theme.px(480)
    x: Screen.virtualX + (Screen.width - width) / 2
    y: Screen.virtualY + (Screen.height - height) / 3
    visible: true
    color: Theme.bgBase
    title: qsTr("Move %1 to Applications").arg(Brand.productName)

    function move() {
        busy = true
        error = ""
        copyLater.start()
    }
    Timer {
        id: copyLater
        interval: 60
        onTriggered: {
            const err = Platform.moveToApplications()
            if (err.length > 0) {
                win.busy = false
                win.error = err
                Announcer.announce(errorLine, err, true)
                return
            }
            Daemon.call("daemon.quit", {})
            Qt.callLater(Qt.quit)
        }
    }

    SequentialAnimation {
        running: !Motion.reduced && win.visible
        loops: Animation.Infinite
        PauseAnimation { duration: 500 }
        NumberAnimation { target: win; property: "travel"; from: 0; to: 1; duration: 1700; easing.type: Easing.InOutCubic }
        NumberAnimation { target: win; property: "folderBump"; from: 1; to: 1.1; duration: 140; easing.type: Easing.OutQuad }
        NumberAnimation { target: win; property: "folderBump"; to: 1; duration: 220; easing.type: Easing.OutBack }
        PauseAnimation { duration: 1100 }
        PropertyAction { target: win; property: "travel"; value: 0 }
    }

    ColumnLayout {
        id: column
        anchors.fill: parent
        anchors.margins: Theme.s6
        spacing: Theme.s4

        Item {
            id: stage
            Layout.fillWidth: true
            Layout.preferredHeight: Theme.px(190)
            Accessible.role: Accessible.Graphic
            Accessible.name: qsTr("The %1 icon moves into the Applications folder.").arg(Brand.productName)
            readonly property real iconSize: Theme.px(88)
            readonly property point from: Qt.point(width * 0.2, height * 0.58)
            readonly property point to: Qt.point(width * 0.8, height * 0.58)
            readonly property point control: Qt.point(width * 0.5, height * 0.02)
            function along(t) {
                const m = 1 - t
                return Qt.point(m * m * from.x + 2 * m * t * control.x + t * t * to.x, m * m * from.y + 2 * m * t * control.y + t * t * to.y)
            }

            Repeater {
                model: 11
                Rectangle {
                    required property int index
                    readonly property point at: stage.along(0.12 + index * 0.076)
                    width: 6
                    height: 6
                    radius: 3
                    x: at.x - 3
                    y: at.y - stage.iconSize * 0.62
                    color: Theme.borderStrong
                    opacity: 0.7
                }
            }

            Image {
                x: stage.from.x - width / 2
                y: stage.from.y - height / 2
                width: stage.iconSize
                height: width
                source: "qrc:/brand/setu.svg"
                sourceSize: Qt.size(width * 2, height * 2)
                opacity: 0.35
                Accessible.ignored: true
            }
            Label {
                x: stage.from.x - width / 2
                y: stage.from.y + stage.iconSize / 2 + Theme.s1
                text: Brand.productName
                role: "small"
                color: Theme.textSecondary
            }

            Item {
                id: folder
                x: stage.to.x - width / 2
                y: stage.to.y - height / 2
                width: stage.iconSize * 1.15
                height: stage.iconSize * 0.9
                scale: win.folderBump
                Rectangle {
                    x: width * 0.06
                    y: 0
                    width: parent.width * 0.42
                    height: parent.height * 0.3
                    radius: Theme.radiusPanel
                    color: Theme.info
                    opacity: 0.75
                }
                Rectangle {
                    y: parent.height * 0.14
                    width: parent.width
                    height: parent.height * 0.86
                    radius: Theme.radiusPanel
                    color: Theme.info
                    Label {
                        anchors.centerIn: parent
                        text: qsTr("A")
                        role: "display"
                        color: Theme.infoBg
                        Accessible.ignored: true
                    }
                }
                Icon {
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    anchors.margins: -Theme.s2
                    name: "circle-check"
                    size: Theme.px(28)
                    color: Theme.success
                    visible: win.travel >= 1 || Motion.reduced
                }
            }
            Label {
                x: stage.to.x - width / 2
                y: stage.to.y + stage.iconSize / 2 + Theme.s1
                text: qsTr("Applications")
                role: "small"
                color: Theme.textSecondary
            }

            Item {
                id: ghost
                readonly property point at: stage.along(win.travel)
                x: at.x - width / 2
                y: at.y - height / 2
                width: stage.iconSize
                height: width
                scale: 1 - 0.45 * Math.max(0, (win.travel - 0.7) / 0.3)
                opacity: win.travel >= 1 ? 0 : 1
                Image {
                    anchors.fill: parent
                    source: "qrc:/brand/setu.svg"
                    sourceSize: Qt.size(width * 2, height * 2)
                    Accessible.ignored: true
                }
                Canvas {
                    x: parent.width * 0.62
                    y: parent.height * 0.6
                    width: Theme.px(22)
                    height: Theme.px(30)
                    onPaint: {
                        const c = getContext("2d")
                        const w = width, h = height
                        c.reset()
                        c.beginPath()
                        c.moveTo(1, 1)
                        c.lineTo(1, h * 0.8)
                        c.lineTo(w * 0.32, h * 0.62)
                        c.lineTo(w * 0.52, h - 1)
                        c.lineTo(w * 0.7, h * 0.92)
                        c.lineTo(w * 0.5, h * 0.56)
                        c.lineTo(w - 1, h * 0.56)
                        c.closePath()
                        c.fillStyle = Theme.textPrimary
                        c.strokeStyle = Theme.bgBase
                        c.lineWidth = 1.5
                        c.fill()
                        c.stroke()
                    }
                }
            }
        }

        Label {
            Layout.fillWidth: true
            text: qsTr("Move %1 to Applications").arg(Brand.productName)
            role: "title"
            Accessible.role: Accessible.Heading
        }
        Label {
            Layout.fillWidth: true
            text: qsTr("%1 is running from the disk image. Drag the %1 icon onto the Applications folder in the Finder window, or let %1 move itself. It then starts from Applications, and you can eject the disk image.").arg(Brand.productName)
            color: Theme.textSecondary
        }
        Label {
            id: errorLine
            Layout.fillWidth: true
            visible: win.error.length > 0
            text: win.error
            color: Theme.danger
            Accessible.role: Accessible.AlertMessage
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s2
            Item {
                Layout.fillWidth: true
            }
            Button {
                variant: "quiet"
                text: qsTr("Not now")
                enabled: !win.busy
                onClicked: win.dismissed()
            }
            Button {
                id: moveButton
                variant: "primary"
                text: qsTr("Move to Applications")
                busy: win.busy
                focus: true
                onClicked: win.move()
            }
        }
    }
    Shortcut {
        sequences: [StandardKey.Cancel]
        enabled: !win.busy
        onActivated: win.dismissed()
    }
    Component.onCompleted: moveButton.forceActiveFocus()
}
