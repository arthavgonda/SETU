import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme

Rectangle {
    id: root
    property string level: "info"
    property string text
    property string actionText
    property string secondaryText
    property bool dismissible: actionText.length === 0
    property bool announce: true
    signal action
    signal secondary
    signal dismissed
    implicitHeight: body.implicitHeight + Theme.s3 * 2
    implicitWidth: 480
    radius: Theme.radiusPanel
    color: Theme.statusBg(level)
    border.width: Theme.border
    border.color: Theme.statusColor(level)
    Accessible.role: level === "danger" ? Accessible.AlertMessage : Accessible.Grouping
    Accessible.name: text

    Component.onCompleted: if (announce && visible && text.length > 0) Announcer.announce(root, text, level === "danger")
    onTextChanged: if (announce && visible && text.length > 0) Announcer.announce(root, text, level === "danger")

    RowLayout {
        id: body
        anchors.fill: parent
        anchors.margins: Theme.s3
        spacing: Theme.s3
        Icon {
            name: Theme.statusIcon(root.level)
            color: Theme.statusColor(root.level)
            Layout.alignment: Qt.AlignTop
        }
        Label {
            Layout.fillWidth: true
            text: root.text
        }
        Button {
            visible: root.actionText.length > 0
            text: root.actionText
            onClicked: root.action()
        }
        Button {
            visible: root.secondaryText.length > 0
            text: root.secondaryText
            variant: "quiet"
            onClicked: root.secondary()
        }
        IconButton {
            visible: root.dismissible
            label: qsTr("Dismiss")
            iconName: "x"
            onClicked: root.dismissed()
        }
    }
}
