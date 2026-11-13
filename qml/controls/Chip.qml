import QtQuick
import QtQuick.Templates as T
import QtQuick.Layouts
import Setu.Theme

T.AbstractButton {
    id: chip
    property bool removable: false
    property bool mono: false
    signal removeRequested
    implicitHeight: Math.max(Theme.hit, chipLabel.implicitHeight + Theme.s1 * 2)
    implicitWidth: chipRow.implicitWidth + leftPadding + rightPadding
    leftPadding: Theme.s3
    rightPadding: removable ? Theme.s1 : Theme.s3
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    scale: down && !Motion.reduced ? 0.94 : 1
    Behavior on scale {
        NumberAnimation { duration: Motion.spring; easing.type: Easing.OutBack; easing.overshoot: Motion.overshoot * 2 }
    }
    Accessible.role: Accessible.Button
    Accessible.name: text
    Keys.onDeletePressed: if (removable) removeRequested()
    Keys.onPressed: event => {
        if (removable && event.key === Qt.Key_Backspace) {
            removeRequested()
            event.accepted = true
        }
    }
    contentItem: RowLayout {
        id: chipRow
        spacing: Theme.s1
        Label {
            id: chipLabel
            text: chip.text
            role: chip.mono ? "mono" : "label"
            wrapMode: Text.NoWrap
        }
        IconButton {
            visible: chip.removable
            label: qsTr("Remove %1").arg(chip.text)
            iconName: "x"
            implicitWidth: Theme.hit - Theme.s2
            focusPolicy: Qt.NoFocus
            onClicked: chip.removeRequested()
        }
    }
    background: Rectangle {
        radius: Theme.radiusControl
        color: chip.down ? Theme.bgPressed : chip.hovered ? Theme.bgHover : Theme.bgSunken
        border.width: Theme.border
        border.color: Theme.borderStrong
        FocusRing {
            shown: chip.visualFocus
        }
    }
}
