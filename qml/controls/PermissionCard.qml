import QtQuick
import QtQuick.Layouts
import Setu.Theme

Rectangle {
    id: card
    property string name
    property string allows
    property string never
    property string status
    property string fix
    property bool required: false
    signal requested(string fix)
    implicitHeight: col.implicitHeight + Theme.s4 * 2
    implicitWidth: 520
    radius: Theme.radiusPanel
    color: Theme.bgRaised
    border.width: Theme.border
    border.color: Theme.borderSubtle
    Accessible.role: Accessible.Grouping
    Accessible.name: name + ", " + statusText

    readonly property string statusText: {
        switch (status) {
        case "granted": return qsTr("Granted")
        case "not_granted": return qsTr("Not granted")
        case "needs_restart": return qsTr("Needs restart of the app")
        case "ask_each_time": return qsTr("Asked each time")
        case "unsupported": return qsTr("Not available on this computer")
        default: return qsTr("Unknown")
        }
    }
    readonly property string level: status === "granted" ? "success" : status === "not_granted" ? (required ? "danger" : "warning") : "info"
    readonly property string actionText: {
        switch (fix) {
        case "allow": return qsTr("Allow")
        case "open_settings": return qsTr("Open system settings")
        case "revoke": return qsTr("Revoke")
        default: return ""
        }
    }

    ColumnLayout {
        id: col
        anchors.fill: parent
        anchors.margins: Theme.s4
        spacing: Theme.s2
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s3
            Label {
                Layout.fillWidth: true
                text: card.required ? qsTr("%1 (required)").arg(card.name) : card.name
                role: "heading"
            }
            StatusDot {
                level: card.level
                text: card.statusText
                showIcon: true
            }
        }
        Label {
            Layout.fillWidth: true
            text: qsTr("Allows: %1").arg(card.allows)
        }
        Label {
            Layout.fillWidth: true
            text: qsTr("Never: %1").arg(card.never)
            color: Theme.textSecondary
        }
        Button {
            visible: card.actionText.length > 0
            text: card.actionText
            variant: card.fix === "revoke" ? "secondary" : "secondary"
            onClicked: card.requested(card.fix)
        }
    }
}
