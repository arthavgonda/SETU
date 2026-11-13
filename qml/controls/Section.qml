import QtQuick
import QtQuick.Layouts
import Setu.Theme

ColumnLayout {
    id: root
    property string title
    property string description
    default property alias content: inner.data
    spacing: Theme.s2
    Label {
        Layout.fillWidth: true
        visible: text.length > 0
        text: root.title
        role: "heading"
        Accessible.role: Accessible.Heading
    }
    Label {
        Layout.fillWidth: true
        visible: text.length > 0
        text: root.description
        color: Theme.textSecondary
        role: "small"
    }
    ColumnLayout {
        id: inner
        Layout.fillWidth: true
        spacing: Theme.s2
    }
    Rectangle {
        Layout.fillWidth: true
        Layout.topMargin: Theme.s2
        implicitHeight: 1
        color: Theme.borderSubtle
        Accessible.ignored: true
    }
}
