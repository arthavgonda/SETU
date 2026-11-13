import QtQuick
import QtQuick.Templates as T
import QtQuick.Layouts
import Setu.Theme

T.AbstractButton {
    id: pill
    property bool selected: false
    property string detail
    property string iconName
    leftPadding: Theme.s3
    rightPadding: Theme.s3
    topPadding: Theme.s1
    bottomPadding: Theme.s1
    implicitHeight: Math.max(Theme.hit + Theme.s1, implicitContentHeight + topPadding + bottomPadding)
    implicitWidth: implicitContentWidth + leftPadding + rightPadding
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    scale: down && !Motion.reduced ? 0.93 : 1
    Behavior on scale {
        NumberAnimation { duration: Motion.spring; easing.type: Easing.OutBack; easing.overshoot: Motion.overshoot * 2 }
    }
    Accessible.role: Accessible.RadioButton
    Accessible.name: detail.length > 0 ? text + ", " + detail : text
    Accessible.checkable: true
    Accessible.checked: selected
    Accessible.onPressAction: pill.clicked()
    Keys.onSpacePressed: pill.clicked()
    Keys.onReturnPressed: pill.clicked()
    contentItem: RowLayout {
        id: row
        spacing: Theme.s2
        Icon {
            visible: pill.iconName.length > 0
            name: pill.iconName
            size: Theme.px(16)
            color: pill.selected ? Theme.textOnAccent : Theme.textSecondary
        }
        Label {
            text: pill.text
            role: "label"
            wrapMode: Text.NoWrap
            color: pill.selected ? Theme.textOnAccent : Theme.textPrimary
            font.weight: pill.selected ? Font.DemiBold : Font.Medium
        }
        Label {
            visible: pill.detail.length > 0
            text: pill.detail
            role: "small"
            wrapMode: Text.NoWrap
            color: pill.selected ? Theme.textOnAccent : Theme.textSecondary
            opacity: pill.selected ? 0.85 : 1
        }
    }
    background: Rectangle {
        radius: height / 2
        color: pill.selected ? Theme.accent : pill.down ? Theme.bgPressed : pill.hovered ? Theme.bgHover : Theme.bgSunken
        border.width: pill.selected ? 0 : Theme.border
        border.color: Theme.borderSubtle
        Behavior on color {
            ColorAnimation { duration: Motion.standard }
        }
        FocusRing {
            shown: pill.visualFocus
        }
    }
}
