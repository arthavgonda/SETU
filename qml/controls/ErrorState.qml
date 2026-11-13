import QtQuick
import QtQuick.Layouts
import Setu.Theme

RowLayout {
    id: root
    property string text
    property string actionText: qsTr("Try again")
    signal action
    spacing: Theme.s2
    Icon {
        name: "circle-alert"
        color: Theme.danger
        Layout.alignment: Qt.AlignTop
    }
    Label {
        Layout.fillWidth: true
        text: root.text
    }
    Button {
        text: root.actionText
        visible: root.actionText.length > 0
        onClicked: root.action()
    }
}
