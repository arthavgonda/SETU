import QtQuick
import QtQuick.Layouts
import Setu.Theme
import Ledger

FocusScope {
    id: page
    property string title
    property string description
    property bool showSaved: true
    property bool embedded: false
    property alias scroll: scroller
    default property alias content: holder.data
    signal navigate(string page)
    signal toast(string text)

    Scroll {
        id: scroller
        anchors.fill: parent
        gutter: Theme.s6
        PageHeader {
            width: parent.width
            visible: !page.embedded
            title: page.title
            description: page.description
            showSaved: page.showSaved
        }
        ColumnLayout {
            id: holder
            width: parent.width
            spacing: Theme.s6
        }
    }
}
