import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Setu.ShellHost
import Ledger

Window {
    id: win
    property var shown: ({})
    property real left: 1
    readonly property bool calm: Motion.reduced
    visible: true
    color: "transparent"
    width: card.width + 16
    height: card.height + 16
    title: qsTr("Saved password")
    Component.onCompleted: {
        OverlayHost.prepare(win, false)
        OverlayHost.place(win, width, height, 24 + Theme.px(260))
        OverlayHost.setInputRegion(win, [Qt.rect(0, 0, width, height)])
        drain.start()
    }

    NumberAnimation {
        id: drain
        target: win
        property: "left"
        from: 1
        to: 0
        duration: Math.max(1, (win.shown.seconds || 10) * 1000)
    }

    Rectangle {
        id: card
        x: 8
        y: 8
        width: Theme.px(360)
        height: col.implicitHeight + Theme.px(36)
        radius: Theme.px(24)
        color: Theme.notchBg
        border.width: 1
        border.color: Theme.notchRaised
        Accessible.role: Accessible.Pane
        Accessible.name: qsTr("Your %1 password, shown for a few seconds").arg(win.shown.title || "")
        scale: up ? 1 : 0.9
        opacity: up ? 1 : 0
        property bool up: false
        Component.onCompleted: up = true
        Behavior on scale {
            NumberAnimation { duration: win.calm ? 0 : 300; easing.type: Easing.OutBack }
        }
        Behavior on opacity {
            NumberAnimation { duration: win.calm ? 0 : 160 }
        }

        ColumnLayout {
            id: col
            anchors.top: parent.top
            anchors.topMargin: Theme.px(18)
            anchors.horizontalCenter: parent.horizontalCenter
            width: parent.width - Theme.px(36)
            spacing: Theme.px(10)

            Text {
                Layout.fillWidth: true
                text: (win.shown.title || "") + (win.shown.account ? "  ·  " + win.shown.account : "")
                elide: Text.ElideRight
                color: Theme.notchMuted
                font.family: Type.sans
                font.pixelSize: Theme.px(13)
            }
            TextEdit {
                Layout.fillWidth: true
                readOnly: true
                selectByMouse: true
                wrapMode: TextEdit.WrapAnywhere
                text: win.shown.password || ""
                color: Theme.notchInk
                selectionColor: Theme.notchListenDeep
                font.family: Type.mono
                font.pixelSize: Theme.px(20)
                Accessible.ignored: true
            }
            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: Theme.px(4)
                radius: height / 2
                color: Theme.notchRaised
                Rectangle {
                    width: parent.width * win.left
                    height: parent.height
                    radius: height / 2
                    gradient: Gradient {
                        orientation: Gradient.Horizontal
                        GradientStop { position: 0; color: Theme.notchListenDeep }
                        GradientStop { position: 1; color: Theme.notchSpeak }
                    }
                }
            }
            RowLayout {
                Layout.fillWidth: true
                Item { Layout.fillWidth: true }
                Rectangle {
                    Layout.preferredWidth: Theme.px(96)
                    Layout.preferredHeight: Theme.px(34)
                    radius: height / 2
                    color: hideTap.pressed ? Theme.notchMuted : Theme.notchInk
                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("Hide")
                    Accessible.onPressAction: Daemon.call("vault.hide", {})
                    Text {
                        anchors.centerIn: parent
                        text: qsTr("Hide")
                        color: Theme.notchBg
                        font.family: Type.sans
                        font.pixelSize: Theme.px(14)
                        font.weight: Font.DemiBold
                    }
                    TapHandler {
                        id: hideTap
                        onTapped: Daemon.call("vault.hide", {})
                    }
                    HoverHandler {
                        cursorShape: Qt.PointingHandCursor
                    }
                }
            }
        }
    }
}
