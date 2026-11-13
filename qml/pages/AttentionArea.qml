import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

FocusScope {
    id: root
    readonly property bool hasContent: Daemon.attention.length > 0
    signal openPage(string page)
    implicitHeight: col.implicitHeight
    activeFocusOnTab: false

    function run(a) {
        if (a.method === "ui.open")
            root.openPage(a.params.page)
        else if (a.method === "ui.openPath")
            Platform.openPath(a.params.path)
        else
            Daemon.call(a.method, a.params || {})
    }

    ColumnLayout {
        id: col
        anchors.left: parent.left
        anchors.right: parent.right
        spacing: Theme.s2
        Repeater {
            model: Daemon.attention
            Banner {
                required property var modelData
                Layout.fillWidth: true
                level: modelData.level
                text: modelData.message
                actionText: modelData.actions.length > 0 ? modelData.actions[0].label : ""
                secondaryText: modelData.actions.length > 1 ? modelData.actions[1].label : ""
                dismissible: false
                onAction: root.run(modelData.actions[0])
                onSecondary: root.run(modelData.actions[1])
            }
        }
    }
}
