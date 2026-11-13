import QtQuick
import QtQuick.Templates as T
import Setu.Theme

T.Button {
    id: control
    required property string label
    property string iconName
    property string shortcutText: ""
    property bool danger: false
    property color ink: danger ? Theme.danger : Theme.textPrimary
    implicitWidth: Math.max(Theme.hit, Theme.controlHeight)
    implicitHeight: implicitWidth
    padding: 0
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    Keys.onReturnPressed: event => {
        control.clicked()
        event.accepted = true
    }
    Keys.onEnterPressed: event => {
        control.clicked()
        event.accepted = true
    }
    Accessible.role: Accessible.Button
    Accessible.name: label
    Accessible.description: shortcutText
    Accessible.onPressAction: control.clicked()

    contentItem: Item {
        Icon {
            anchors.centerIn: parent
            name: control.iconName
            color: !control.enabled ? Theme.textTertiary : control.ink
        }
    }
    background: Rectangle {
        radius: Theme.radiusControl
        color: control.down ? Theme.bgPressed : control.hovered ? Theme.bgHover : "transparent"
        border.width: Theme.contrast && (control.hovered || control.visualFocus) ? Theme.border : 0
        border.color: Theme.borderStrong
        FocusRing {
            shown: control.visualFocus
        }
    }
}
