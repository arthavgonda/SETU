import QtQuick
import QtQuick.Templates as T
import Setu.Theme

T.RadioButton {
    id: control
    property string description: ""
    implicitWidth: Math.max(Theme.hit, leftPadding + rightPadding + texts.leftPadding + Math.max(mainText.implicitWidth, descText.visible ? Math.min(descText.implicitWidth, Theme.px(480)) : 0))
    implicitHeight: Math.max(Theme.hit + Theme.s1, texts.implicitHeight + topPadding + bottomPadding)
    padding: Theme.s1
    spacing: Theme.s3
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    Accessible.role: Accessible.RadioButton
    Accessible.name: text
    Accessible.description: description

    indicator: Rectangle {
        x: control.leftPadding
        y: control.topPadding + Theme.px(1)
        implicitWidth: Theme.px(18)
        implicitHeight: Theme.px(18)
        radius: width / 2
        color: Theme.bgSunken
        border.width: Math.max(Theme.border, 1)
        border.color: control.checked ? Theme.accent : Theme.borderStrong
        Rectangle {
            anchors.centerIn: parent
            width: parent.width / 2
            height: width
            radius: width / 2
            color: Theme.accent
            visible: control.checked
        }
        FocusRing {
            shown: control.visualFocus
            radiusBase: width / 2
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
