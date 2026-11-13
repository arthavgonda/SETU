import QtQuick
import QtQuick.Templates as T
import Setu.Core
import Setu.Theme
import Ledger

T.Button {
    id: control
    property string variant: "secondary"
    property string iconName: ""
    property color tint: Theme.notchInk
    property bool round: text.length === 0
    property bool stopGlyph: false
    readonly property color face: variant === "primary" ? Theme.notchInk : variant === "danger" ? Theme.notchDanger : variant === "focus" ? tint : Theme.notchRaised
    readonly property color ink: variant === "primary" ? Theme.notchBg : Theme.notchInk
    implicitHeight: Theme.px(34)
    implicitWidth: round ? implicitHeight : Math.max(implicitHeight * 2, row.implicitWidth + Theme.px(28))
    focusPolicy: Qt.NoFocus
    hoverEnabled: true
    scale: down && !Motion.reduced ? 0.9 : hovered && !Motion.reduced ? 1.04 : 1
    Behavior on scale {
        NumberAnimation { duration: Motion.spring; easing.type: Easing.OutBack; easing.overshoot: 3 }
    }
    Accessible.role: Accessible.Button
    Accessible.name: text.length > 0 ? text : Accessible.name
    Accessible.onPressAction: clicked()
    onClicked: Daemon.call("sound.play", { name: "tap" })

    contentItem: Item {
        implicitWidth: row.implicitWidth
        implicitHeight: row.implicitHeight
        Row {
            id: row
            anchors.centerIn: parent
            spacing: Theme.px(6)
            Rectangle {
                anchors.verticalCenter: parent.verticalCenter
                visible: control.stopGlyph
                width: Math.round(control.implicitHeight * 0.32)
                height: width
                radius: Math.round(width * 0.22)
                color: control.ink
            }
            Icon {
                anchors.verticalCenter: parent.verticalCenter
                name: control.iconName
                color: control.ink
                size: Math.round(control.implicitHeight * 0.5)
            }
            Text {
                anchors.verticalCenter: parent.verticalCenter
                visible: control.text.length > 0
                text: control.text
                color: control.ink
                font.family: Type.sans
                font.pixelSize: Theme.px(14)
                font.weight: Font.DemiBold
                textFormat: Text.PlainText
                renderType: Text.NativeRendering
            }
        }
    }
    background: Rectangle {
        radius: height / 2
        color: control.down ? Qt.darker(control.face, control.variant === "primary" ? 1.12 : 1.25) : control.hovered ? Qt.lighter(control.face, control.variant === "primary" ? 1.0 : 1.25) : control.face
        border.width: control.visualFocus ? 2 : 0
        border.color: Theme.notchInk
        Behavior on color {
            ColorAnimation { duration: 120 }
        }
    }
}
