import QtQuick
import QtQuick.Templates as T
import Setu.Theme

T.Switch {
    id: control
    property string description: ""
    implicitWidth: Math.max(Theme.hit, leftPadding + rightPadding + texts.leftPadding + Math.max(mainText.implicitWidth, descText.visible ? Math.min(descText.implicitWidth, Theme.px(480)) : 0))
    implicitHeight: Math.max(Theme.rowHeight - Theme.s2, texts.implicitHeight + topPadding + bottomPadding)
    padding: Theme.s1
    spacing: Theme.s3
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    Accessible.role: Accessible.CheckBox
    Accessible.name: text
    Accessible.description: (checked ? qsTr("On") : qsTr("Off")) + (description.length > 0 ? ". " + description : "")
    Accessible.checkable: true
    Accessible.checked: checked

    indicator: Rectangle {
        x: control.leftPadding
        y: control.topPadding + Theme.px(2)
        width: Theme.px(36)
        height: Theme.px(20)
        radius: height / 2
        color: control.checked ? Theme.accent : Theme.bgSunken
        border.width: control.checked && !Theme.contrast ? 0 : Theme.border
        border.color: control.enabled ? Theme.borderStrong : Theme.borderSubtle
        opacity: control.enabled ? 1 : 0.5
        Rectangle {
            width: parent.height - 6
            height: width
            radius: width / 2
            y: 3
            x: control.checked ? parent.width - width - 3 : 3
            color: control.checked ? Theme.textOnAccent : Theme.textSecondary
            Behavior on x {
                NumberAnimation {
                    duration: Motion.standard
                    easing.type: Easing.Bezier
                    easing.bezierCurve: Motion.curve
                }
            }
        }
        FocusRing {
            shown: control.visualFocus
            radiusBase: height / 2
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
