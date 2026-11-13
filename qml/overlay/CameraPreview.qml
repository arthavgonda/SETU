import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Setu.ShellHost
import Ledger

Window {
    id: win
    property string frame
    property int secondsLeft: 60
    visible: true
    color: "transparent"
    width: Theme.px(480) + 16
    height: col.implicitHeight + 16
    title: qsTr("Camera on")
    Component.onCompleted: {
        OverlayHost.prepare(win, false)
        OverlayHost.place(win, width, height, 24 + Theme.px(260))
        OverlayHost.setInputRegion(win, [Qt.rect(0, 0, width, height)])
        Announcer.announce(box, hint.text)
    }
    Rectangle {
        LayoutMirroring.enabled: Platform.rightToLeft
        LayoutMirroring.childrenInherit: true
        id: box
        anchors.fill: parent
        anchors.margins: 8
        radius: Theme.radiusOverlay
        color: Theme.bgRaised
        border.width: Theme.border
        border.color: Theme.borderStrong
        Accessible.role: Accessible.Pane
        Accessible.name: win.title
        ColumnLayout {
            id: col
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.margins: Theme.s3
            spacing: Theme.s2
            RowLayout {
                Rectangle {
                    Layout.preferredWidth: 8
                    Layout.preferredHeight: 8
                    radius: Theme.radiusRound
                    color: Theme.danger
                }
                Label {
                    Layout.fillWidth: true
                    text: qsTr("Camera on")
                    font.weight: Font.DemiBold
                }
                Label {
                    text: qsTr("Closes in %n s", "", win.secondsLeft)
                    role: "small"
                    color: Theme.textSecondary
                }
            }
            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: width * 3 / 4
                color: Theme.bgSunken
                radius: Theme.radiusControl
                Image {
                    anchors.fill: parent
                    fillMode: Image.PreserveAspectFit
                    source: win.frame.length > 0 ? "data:image/jpeg;base64," + win.frame : ""
                    cache: false
                    asynchronous: false
                    Accessible.ignored: true
                }
            }
            Label {
                id: hint
                Layout.fillWidth: true
                text: qsTr("Camera on. Say click to take the photo. Say cancel to close.")
            }
            RowLayout {
                spacing: Theme.s2
                Button {
                    text: qsTr("Take photo")
                    variant: "primary"
                    focusPolicy: Qt.NoFocus
                    onClicked: Daemon.call("camera.capture", {})
                }
                Button {
                    text: qsTr("Close camera")
                    focusPolicy: Qt.NoFocus
                    onClicked: Daemon.call("camera.close", {})
                }
            }
        }
    }
}
