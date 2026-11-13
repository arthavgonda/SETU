import QtQuick
import QtQuick.Templates as T
import QtQuick.Layouts
import Setu.Theme

ColumnLayout {
    id: root
    property string title
    property bool expanded: false
    default property alias content: inner.data
    spacing: Theme.s2
    T.AbstractButton {
        id: head
        Layout.fillWidth: true
        implicitHeight: Math.max(Theme.hit + Theme.s1, headRow.implicitHeight + Theme.s2)
        hoverEnabled: true
        focusPolicy: Qt.StrongFocus
        Accessible.role: Accessible.Button
        Accessible.name: root.title
        Accessible.description: root.expanded ? qsTr("Expanded") : qsTr("Collapsed")
        onClicked: root.expanded = !root.expanded
        contentItem: RowLayout {
            id: headRow
            spacing: Theme.s2
            Icon {
                name: root.expanded ? "chevron-down" : "chevron-right"
            }
            Label {
                Layout.fillWidth: true
                text: root.title
                role: "label"
            }
        }
        background: Rectangle {
            radius: Theme.radiusControl
            color: head.hovered ? Theme.bgHover : "transparent"
            FocusRing {
                shown: head.visualFocus
            }
        }
    }
    ColumnLayout {
        id: inner
        visible: root.expanded
        Layout.fillWidth: true
        Layout.leftMargin: Theme.s5
        spacing: Theme.s3
    }
}
