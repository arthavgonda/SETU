import QtQuick
import QtQuick.Templates as T
import Setu.Theme

T.CheckBox {
    id: control
    property string description: ""
    implicitWidth: Math.max(Theme.hit, leftPadding + rightPadding + texts.leftPadding + Math.max(mainText.implicitWidth, descText.visible ? Math.min(descText.implicitWidth, Theme.px(480)) : 0))
    implicitHeight: Math.max(Theme.hit + Theme.s1, texts.implicitHeight + topPadding + bottomPadding)
    padding: Theme.s1
    spacing: Theme.s3
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    Accessible.role: Accessible.CheckBox
    Accessible.name: text
    Accessible.description: description

    indicator: Rectangle {
        x: control.leftPadding
        y: control.topPadding + Theme.px(1)
        implicitWidth: Theme.px(18)
        implicitHeight: Theme.px(18)
        radius: Theme.radiusControl
        color: control.checked ? Theme.accent : Theme.bgSunken
        border.width: Math.max(Theme.border, control.checked ? 0 : 1)
        border.color: Theme.borderStrong
        opacity: control.enabled ? 1 : 0.5
        Icon {
            anchors.centerIn: parent
            name: control.checked ? "check" : ""
            size: Theme.px(14)
            color: Theme.textOnAccent
        }
        FocusRing {
            shown: control.visualFocus
        }
    }
    contentItem: Column {
        id: texts
        leftPadding: control.indicator.width + control.spacing
        spacing: 2
        Label {
            id: mainText
            width: control.availableWidth - texts.leftPadding
            text: control.text
            color: control.enabled ? Theme.textPrimary : Theme.textTertiary
        }
        Label {
            id: descText
            width: control.availableWidth - texts.leftPadding
            visible: control.description.length > 0
            text: control.description
            role: "small"
            color: Theme.textSecondary
        }
    }
    background: Rectangle {
        radius: Theme.radiusControl
        color: control.hovered ? Theme.bgHover : "transparent"
    }
}
