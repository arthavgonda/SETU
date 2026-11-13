import QtQuick
import QtQuick.Layouts
import Setu.Theme
import Ledger

FocusScope {
    id: page
    property string title
    property string description
    property bool showSaved: true
    default property alias content: holder.data
    signal navigate(string page)
    signal toast(string text)

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.s6
        spacing: Theme.s5
        PageHeader {
            Layout.fillWidth: true
            title: page.title
            description: page.description
            showSaved: page.showSaved
        }
        ColumnLayout {
            id: holder
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: Theme.s4
        }
    }
}
