import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme

ColumnLayout {
    id: root
    property string title
    property string description
    property bool showSaved: true
    property bool savedVisible: false
    property string rejectedText
    spacing: Theme.s1

    Connections {
        target: Settings
        function onSaved() {
            if (root.showSaved) {
                root.savedVisible = true
                hide.restart()
            }
        }
        function onRejected(key, message) {
            root.rejectedText = message
        }
    }
    Timer {
        id: hide
        interval: 3000
        onTriggered: root.savedVisible = false
    }
    RowLayout {
        Layout.fillWidth: true
        spacing: Theme.s3
        Label {
            Layout.fillWidth: true
            text: root.title
            role: "title"
            Accessible.role: Accessible.Heading
        }
        Row {
            spacing: Theme.s1
            visible: root.savedVisible
            Icon {
                name: "check"
                color: Theme.success
                size: Theme.px(16)
                anchors.verticalCenter: parent.verticalCenter
            }
            Label {
                text: qsTr("Saved")
                role: "small"
                color: Theme.textSecondary
                anchors.verticalCenter: parent.verticalCenter
            }
        }
    }
    Label {
        Layout.fillWidth: true
        visible: text.length > 0
        text: root.description
        color: Theme.textSecondary
    }
    Banner {
        Layout.fillWidth: true
        Layout.topMargin: Theme.s2
        visible: root.rejectedText.length > 0
        level: "danger"
        text: root.rejectedText
        onDismissed: root.rejectedText = ""
    }
}
