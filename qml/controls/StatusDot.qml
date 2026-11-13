import QtQuick
import QtQuick.Layouts
import Setu.Theme

RowLayout {
    id: root
    property string level: "info"
    property string text
    property bool showIcon: Theme.contrast
    spacing: Theme.s2
    Accessible.role: Accessible.StaticText
    Accessible.name: text
    Item {
        Layout.alignment: Qt.AlignTop
        Layout.preferredWidth: root.showIcon ? Theme.px(16) : 8
        Layout.preferredHeight: label.lineHeight > 0 ? Math.max(Theme.px(16), Math.min(label.implicitHeight, label.lineHeight)) : Theme.px(16)
        Accessible.ignored: true
        Rectangle {
            visible: !root.showIcon
            anchors.centerIn: parent
            width: 8
            height: 8
            radius: Theme.radiusRound
            color: Theme.statusColor(root.level)
        }
        Icon {
            visible: root.showIcon
            anchors.centerIn: parent
            name: Theme.statusIcon(root.level)
            color: Theme.statusColor(root.level)
            size: Theme.px(16)
        }
    }
    Label {
        id: label
        Layout.fillWidth: true
        text: root.text
        Accessible.ignored: true
    }
}
