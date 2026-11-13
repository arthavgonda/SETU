import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme

ColumnLayout {
    id: root
    property string what
    property string slowText: qsTr("This is taking longer than usual. The background service may be busy starting its models.")
    property bool slow: false
    spacing: Theme.s1
    Label {
        text: qsTr("Loading")
        role: "label"
        color: Theme.textSecondary
    }
    Label {
        Layout.fillWidth: true
        visible: root.slow
        text: root.slowText
        role: "small"
        color: Theme.textSecondary
    }
    Timer {
        interval: 3000
        running: root.visible && !root.slow
        onTriggered: root.slow = true
    }
    onVisibleChanged: {
        if (visible) {
            slow = false
            Announcer.announce(root, qsTr("Loading %1").arg(root.what))
        }
    }
}
