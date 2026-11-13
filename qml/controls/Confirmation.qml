import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme

FocusScope {
    id: root
    property string title
    property var facts: []
    property string consequence
    property string actionLabel
    property bool danger: false
    property int timeoutMs: 0
    property bool voiceAllowed: false
    property real remaining: timeoutMs
    property bool compact: false
    property bool actionEnabled: true
    readonly property string method: Settings.values["confirm.method"] ?? "any"
    readonly property bool keysAllowed: method === "key" || method === "any" || method === "voice"
    signal confirmed(string via)
    signal cancelled(string via)
    implicitHeight: col.implicitHeight
    implicitWidth: Theme.px(520)
    Accessible.role: Accessible.AlertMessage
    Accessible.name: title + ". " + consequence

    function start() {
        remaining = timeoutMs
        cancelButton.forceActiveFocus()
        Announcer.announce(root, title + ". " + consequence, true)
    }

    Keys.onEscapePressed: root.cancelled("key")
    Keys.onReturnPressed: event => {
        if (cancelButton.activeFocus)
            root.cancelled("key")
        else if (root.keysAllowed && root.actionEnabled)
            root.confirmed("key")
    }

    Timer {
        interval: 1000
        repeat: true
        running: root.visible && root.timeoutMs > 0 && root.remaining > 0
        onTriggered: root.remaining = Math.max(0, root.remaining - 1000)
    }

    ColumnLayout {
        id: col
        anchors.left: parent.left
        anchors.right: parent.right
        spacing: Theme.s3
        Label {
            Layout.fillWidth: true
            visible: text.length > 0
            text: root.title
            role: "heading"
            Accessible.role: Accessible.Heading
        }
        Rectangle {
            Layout.fillWidth: true
            visible: root.facts.length > 0
            implicitHeight: factCol.implicitHeight + Theme.s3 * 2
            radius: Theme.radiusControl
            color: Theme.bgSunken
            border.width: Theme.border
            border.color: Theme.borderSubtle
            ColumnLayout {
                id: factCol
                anchors.fill: parent
                anchors.margins: Theme.s3
                spacing: Theme.s1
                Repeater {
                    model: root.facts
                    TextEdit {
                        required property string modelData
                        Layout.fillWidth: true
                        text: modelData
                        readOnly: true
                        selectByMouse: true
                        wrapMode: TextEdit.WrapAnywhere
                        textFormat: TextEdit.PlainText
                        font.family: Type.mono
                        font.pixelSize: Type.code.size
                        color: Theme.textPrimary
                        selectionColor: Theme.accent
                        selectedTextColor: Theme.textOnAccent
                        Accessible.role: Accessible.StaticText
                        Accessible.name: modelData
                    }
                }
            }
        }
        Label {
            Layout.fillWidth: true
            text: root.consequence
        }
        Label {
            Layout.fillWidth: true
            role: "small"
            color: Theme.textSecondary
            text: {
                let ways = []
                if (root.voiceAllowed)
                    ways.push(qsTr("say yes or no"))
                if (root.keysAllowed)
                    ways.push(qsTr("press Enter or Esc"))
                ways.push(qsTr("choose a button"))
                const t = root.timeoutMs > 0 ? " " + qsTr("Cancels by itself in %n seconds.", "", Math.ceil(root.remaining / 1000)) : ""
                return qsTr("You can %1.").arg(ways.join(", ")) + t
            }
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s2
            Button {
                id: cancelButton
                text: qsTr("Cancel")
                large: true
                focus: true
                onClicked: root.cancelled("click")
            }
            Item {
                Layout.fillWidth: true
            }
            Button {
                text: root.actionLabel
                variant: root.danger ? "danger" : "primary"
                large: true
                enabled: root.actionEnabled
                onClicked: root.confirmed("click")
            }
        }
    }
}
