import QtQuick
import QtQuick.Layouts
import Setu.Theme

TextField {
    id: root
    property string clearLabel: qsTr("Clear search")
    placeholder: ""
    inputMethodHints: Qt.ImhNoPredictiveText
    leading: Icon {
        name: "search"
        color: Theme.textSecondary
    }
    trailing: IconButton {
        label: root.clearLabel
        iconName: "x"
        visible: root.text.length > 0
        implicitWidth: Theme.hit
        onClicked: {
            root.text = ""
            root.edited()
            root.forceActiveFocus()
        }
    }
    field.Keys.onEscapePressed: event => {
        if (root.text.length > 0) {
            root.text = ""
            root.edited()
            event.accepted = true
        } else {
            event.accepted = false
        }
    }
}
