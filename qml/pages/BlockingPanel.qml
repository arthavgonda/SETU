import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

Rectangle {
    id: root
    property string reason
    property bool tried: false
    property bool failed: false
    property string lastError
    color: Theme.bgBase
    Accessible.role: Accessible.AlertMessage
    Accessible.name: message.text

    onVisibleChanged: if (visible) Announcer.announce(root, message.text, true)

    Timer {
        id: retry
        interval: 1500
        onTriggered: Daemon.reconnectNow()
    }
    Timer {
        id: watch
        interval: 10000
        onTriggered: if (!Daemon.connected) {
            root.failed = true
            root.lastError = Launcher.lastDaemonError()
        }
    }
    Connections {
        target: Daemon
        function onStatusChanged() {
            if (Daemon.connected) {
                root.tried = false
                root.failed = false
            }
        }
    }

    ColumnLayout {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: Theme.s6
        spacing: Theme.s4
        Label {
            Layout.fillWidth: true
            role: "title"
            text: root.reason === "mismatch" ? qsTr("Update needed") : root.failed ? qsTr("Couldn't start") : qsTr("Not running")
            Accessible.role: Accessible.Heading
        }
        Label {
            id: message
            Layout.fillWidth: true
            text: {
                if (root.reason === "mismatch")
                    return qsTr("This version of the app doesn't match the background service. Update %1.").arg(Brand.productName) + (root.lastError.length > 0 ? " " + root.lastError : "")
                if (root.reason === "refused")
                    return qsTr("The background service belongs to another user, so %1 won't connect to it. Sign in as that user, or restart your computer.").arg(Brand.productName)
                if (root.failed)
                    return root.lastError.length > 0 ? qsTr("%1 couldn't start. The last error was: %2.").arg(Brand.productName).arg(root.lastError) : qsTr("%1 couldn't start. No error was recorded.").arg(Brand.productName)
                if (root.tried)
                    return qsTr("Starting %1.").arg(Brand.productName)
                return qsTr("%1 isn't running. Voice control is off.").arg(Brand.productName)
            }
        }
        RowLayout {
            spacing: Theme.s2
            visible: root.reason !== "refused"
            Button {
                visible: root.reason === "mismatch"
                variant: "primary"
                text: qsTr("Update")
                onClicked: {
                    if (Launcher.restartDaemon())
                        retry.restart()
                    else if (Brand.supportUrl.length > 0)
                        Platform.openUrl(Brand.supportUrl)
                    else
                        root.lastError = qsTr("Install the latest version of %1, then open it again.").arg(Brand.productName)
                }
            }
            Button {
                visible: root.reason === "stopped"
                variant: "primary"
                busy: root.tried && !root.failed
                text: root.failed ? qsTr("Try again") : qsTr("Start")
                onClicked: {
                    root.tried = true
                    root.failed = false
                    Daemon.startDaemon()
                    watch.restart()
                }
            }
            Button {
                visible: root.reason === "stopped"
                text: root.failed ? qsTr("Open logs folder") : qsTr("View log")
                onClicked: Platform.openPath(Launcher.logsFolder())
            }
        }
    }
}
