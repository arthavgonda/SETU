import QtQuick
import QtQuick.Layouts
import Setu.Theme

ColumnLayout {
    id: root
    property string text
    property string actionText
    property string secondaryText
    signal action
    signal secondary
    spacing: Theme.s3
    Label {
        Layout.fillWidth: true
        text: root.text
        color: Theme.textSecondary
    }
    RowLayout {
        spacing: Theme.s2
        visible: root.actionText.length > 0
        Button {
            text: root.actionText
            onClicked: root.action()
        }
        Button {
            visible: root.secondaryText.length > 0
            text: root.secondaryText
            variant: "quiet"
            onClicked: root.secondary()
        }
    }
}
