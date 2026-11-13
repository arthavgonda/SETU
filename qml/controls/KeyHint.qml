import QtQuick
import Setu.Theme

Row {
    id: root
    property var keys: []
    property color ink: Theme.textPrimary
    spacing: Theme.s1
    Accessible.role: Accessible.StaticText
    Accessible.name: keys.join(" ")
    Repeater {
        model: root.keys
        Rectangle {
            required property string modelData
            implicitWidth: Math.max(height, cap.implicitWidth + Theme.s2 * 2)
            implicitHeight: cap.implicitHeight + 4
            radius: Theme.radiusControl
            color: Theme.bgSunken
            border.width: Theme.border
            border.color: Theme.borderStrong
            Label {
                id: cap
                anchors.centerIn: parent
                text: parent.modelData
                role: "mono"
                color: root.ink
                wrapMode: Text.NoWrap
            }
        }
    }
}
