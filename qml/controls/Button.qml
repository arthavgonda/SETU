import QtQuick
import QtQuick.Templates as T
import Setu.Theme

T.Button {
    id: control
    property string variant: "secondary"
    property bool filled: false
    property bool busy: false
    property bool large: variant === "primary"
    property string iconName: ""
    property string description: ""
    readonly property bool isDanger: variant === "danger"
    readonly property bool solid: variant === "primary" || (isDanger && filled)

    implicitHeight: large ? Theme.primaryHeight : Theme.controlHeight
    implicitWidth: Math.max(Theme.hit, contentItem.implicitWidth + leftPadding + rightPadding)
    leftPadding: variant === "quiet" ? Theme.s2 : Theme.s4
    rightPadding: leftPadding
    focusPolicy: Qt.StrongFocus
    hoverEnabled: true
    enabled: !busy
    scale: down && !Motion.reduced ? 0.95 : 1
    Behavior on scale {
        NumberAnimation { duration: Motion.spring; easing.type: Easing.OutBack; easing.overshoot: Motion.overshoot * 2 }
    }

    Keys.onReturnPressed: event => {
        control.clicked()
        event.accepted = true
    }
    Keys.onEnterPressed: event => {
        control.clicked()
        event.accepted = true
    }
    Accessible.role: Accessible.Button
    Accessible.name: text
    Accessible.description: busy ? qsTr("Working") : description
    Accessible.onPressAction: control.clicked()

    readonly property color ink: {
        if (!control.enabled && !busy) return Theme.textTertiary
        if (solid) return isDanger ? Theme.textOnDanger : Theme.textOnAccent
        if (isDanger) return Theme.danger
        if (variant === "quiet") return Theme.accent
        return Theme.textPrimary
    }

    contentItem: Item {
        implicitWidth: row.implicitWidth
        implicitHeight: row.implicitHeight
        Row {
            id: row
            anchors.centerIn: parent
            spacing: Theme.s2
            Icon {
                name: control.iconName
                color: control.ink
                anchors.verticalCenter: parent.verticalCenter
            }
            Label {
                text: control.text
                role: "label"
                color: control.ink
                wrapMode: Text.NoWrap
                elide: Text.ElideRight
                anchors.verticalCenter: parent.verticalCenter
                opacity: control.busy ? 0.6 : 1
            }
        }
    }

    background: Rectangle {
        radius: control.large ? Theme.radiusPanel : Theme.radiusControl
        color: {
            if (control.solid) {
                const base = control.isDanger ? Theme.danger : Theme.accent
                if (!control.enabled && !control.busy) return Theme.bgSunken
                return control.down ? (control.isDanger ? Qt.darker(base, 1.15) : Theme.accentPressed) : control.hovered ? (control.isDanger ? Qt.darker(base, 1.07) : Theme.accentHover) : base
            }
            if (control.down) return Theme.bgPressed
            if (control.hovered) return Theme.bgHover
            return control.variant === "quiet" ? "transparent" : Theme.bgRaised
        }
        border.width: control.variant === "quiet" || control.solid ? (Theme.contrast && control.solid ? Theme.border : 0) : Theme.border
        border.color: control.isDanger ? Theme.danger : (control.enabled ? Theme.borderStrong : Theme.borderSubtle)
        Rectangle {
            visible: control.busy
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            anchors.margins: Theme.border
            height: 3
            radius: 1
            color: control.solid ? control.ink : Theme.accent
            opacity: 0.85
            Accessible.ignored: true
        }
        FocusRing {
            shown: control.visualFocus
        }
    }
}
