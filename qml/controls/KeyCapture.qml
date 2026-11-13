import QtQuick
import QtQuick.Templates as T
import QtQuick.Layouts
import Setu.Theme

ColumnLayout {
    id: root
    property string label
    property string sequence: ""
    property bool recording: false
    signal changed(string sequence)
    spacing: Theme.s1

    function keyName(event) {
        const names = { [Qt.Key_Space]: "Space", [Qt.Key_Return]: "Enter", [Qt.Key_Tab]: "Tab", [Qt.Key_Backspace]: "Backspace", [Qt.Key_Delete]: "Delete", [Qt.Key_Home]: "Home", [Qt.Key_End]: "End", [Qt.Key_PageUp]: "PageUp", [Qt.Key_PageDown]: "PageDown", [Qt.Key_Left]: "Left", [Qt.Key_Right]: "Right", [Qt.Key_Up]: "Up", [Qt.Key_Down]: "Down", [Qt.Key_Comma]: "," }
        if (names[event.key])
            return names[event.key]
        if (event.key >= Qt.Key_F1 && event.key <= Qt.Key_F24)
            return "F" + (event.key - Qt.Key_F1 + 1)
        return event.text.length === 1 ? event.text.toUpperCase() : ""
    }

    Label {
        text: root.label
        role: "label"
        visible: text.length > 0
    }
    RowLayout {
        spacing: Theme.s2
        T.AbstractButton {
            id: cap
            Layout.fillWidth: true
            implicitHeight: Theme.controlHeight
            focusPolicy: Qt.StrongFocus
            hoverEnabled: true
            Accessible.role: Accessible.Button
            Accessible.name: root.label
            Accessible.description: root.recording ? qsTr("Press the keys now. Escape cancels.") : (root.sequence.length > 0 ? root.sequence : qsTr("Not set"))
            onClicked: root.recording = true
            Keys.onPressed: event => {
                if (!root.recording)
                    return
                if (event.key === Qt.Key_Escape) {
                    root.recording = false
                    event.accepted = true
                    return
                }
                if ([Qt.Key_Control, Qt.Key_Shift, Qt.Key_Alt, Qt.Key_Meta].includes(event.key))
                    return
                const k = root.keyName(event)
                if (k.length === 0)
                    return
                let parts = []
                if (event.modifiers & Qt.ControlModifier) parts.push("Ctrl")
                if (event.modifiers & Qt.AltModifier) parts.push("Alt")
                if (event.modifiers & Qt.ShiftModifier) parts.push("Shift")
                if (event.modifiers & Qt.MetaModifier) parts.push("Meta")
                parts.push(k)
                root.sequence = parts.join("+")
                root.recording = false
                root.changed(root.sequence)
                event.accepted = true
            }
            contentItem: Item {
                implicitHeight: hint.implicitHeight
                KeyHint {
                    id: hint
                    anchors.verticalCenter: parent.verticalCenter
                    keys: root.sequence.length > 0 && !root.recording ? root.sequence.split("+") : []
                }
                Label {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: root.recording || root.sequence.length === 0
                    text: root.recording ? qsTr("Press the keys. Esc cancels.") : qsTr("Not set")
                    color: Theme.textSecondary
                }
            }
            leftPadding: Theme.s3
            background: Rectangle {
                radius: Theme.radiusControl
                color: Theme.bgSunken
                border.width: root.recording ? 2 : Theme.border
                border.color: root.recording ? Theme.accent : Theme.borderStrong
                FocusRing {
                    shown: cap.visualFocus
                }
            }
        }
        Button {
            text: qsTr("Clear")
            variant: "quiet"
            enabled: root.sequence.length > 0
            onClicked: {
                root.sequence = ""
                root.changed("")
            }
        }
    }
}
