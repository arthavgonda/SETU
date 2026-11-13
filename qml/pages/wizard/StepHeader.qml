import QtQuick
import QtQuick.Layouts
import Setu.Theme
import Ledger

ColumnLayout {
    property string title
    property string subtitle
    spacing: Theme.s2
    Label {
        Layout.fillWidth: true
        horizontalAlignment: Text.AlignHCenter
        text: parent.title
        role: "display"
        Accessible.role: Accessible.Heading
    }
    Label {
        Layout.fillWidth: true
        visible: text.length > 0
        horizontalAlignment: Text.AlignHCenter
        text: parent.subtitle
        color: Theme.textSecondary
    }
}
