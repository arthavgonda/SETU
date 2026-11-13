import QtQuick
import QtQuick.Templates as T
import QtQuick.Layouts
import Setu.Theme

T.MenuItem {
    id: item
    property string iconName
    property string shortcutText
    implicitWidth: row.implicitWidth + leftPadding + rightPadding
    implicitHeight: Math.max(Theme.hit + Theme.s1, row.implicitHeight + Theme.s2 * 2)
    leftPadding: Theme.s3
    rightPadding: Theme.s3
    hoverEnabled: true
    Accessible.role: Accessible.MenuItem
    Accessible.name: text
    contentItem: RowLayout {
        id: row
        spacing: Theme.s3
        Icon {
            name: item.checkable && item.checked ? "check" : item.iconName
            color: item.enabled ? Theme.textPrimary : Theme.textTertiary
        }
        Label {
            Layout.fillWidth: true
            text: item.text
            wrapMode: Text.NoWrap
            color: item.enabled ? Theme.textPrimary : Theme.textTertiary
        }
        KeyHint {
            visible: item.shortcutText.length > 0
            keys: item.shortcutText.length > 0 ? item.shortcutText.split("+") : []
        }
        Icon {
            name: item.subMenu ? "chevron-right" : ""
        }
    }
    background: Rectangle {
        radius: Theme.radiusControl
        color: item.highlighted ? Theme.bgSelected : "transparent"
        border.width: Theme.contrast && item.highlighted ? Theme.border : 0
        border.color: Theme.accent
    }
}
