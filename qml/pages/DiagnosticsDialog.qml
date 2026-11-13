import QtQuick
import QtQuick.Layouts
import QtQuick.Dialogs
import Setu.Core
import Setu.Theme
import Ledger

Dialog {
    id: dlg
    property var sections: []
    property bool loading: false
    property string error
    signal toast(string text)
    title: qsTr("Export diagnostics")
    preferredWidth: Theme.px(760)

    function start() {
        sections = []
        error = ""
        loading = true
        open()
        Daemon.call("diagnostics.preview", {}, (ok, r, e) => {
            dlg.loading = false
            if (ok)
                dlg.sections = r.sections
            else
                dlg.error = e.message
        })
    }

    FileDialog {
        id: saveDialog
        title: qsTr("Save diagnostics")
        fileMode: FileDialog.SaveFile
        nameFilters: [qsTr("Text files (*.txt)")]
        defaultSuffix: "txt"
        onAccepted: Daemon.call("diagnostics.export", { path: Platform.localFile(selectedFile) }, (ok, r, e) => {
            dlg.toast(ok ? qsTr("Saved %1 to %2").arg(Platform.formatBytes(r.bytes)).arg(r.path) : e.message)
            if (ok)
                dlg.close()
        })
    }

    Label {
        Layout.fillWidth: true
        text: qsTr("This is exactly what the file contains. It has no history, no audio and no keys. Nothing is sent anywhere; you choose where to save it.")
        color: Theme.textSecondary
    }
    LoadingState {
        Layout.fillWidth: true
        visible: dlg.loading
        what: dlg.title
    }
    ErrorState {
        Layout.fillWidth: true
        visible: dlg.error.length > 0
        text: dlg.error
        onAction: dlg.start()
    }
    Scroll {
        Layout.fillWidth: true
        Layout.preferredHeight: Theme.px(360)
        visible: dlg.sections.length > 0
        gutter: 0
        spacing: Theme.s3
        Repeater {
            model: dlg.sections
            ColumnLayout {
                required property var modelData
                width: parent.width
                spacing: Theme.s1
                Label {
                    text: modelData.name
                    role: "label"
                    Accessible.role: Accessible.Heading
                }
                TextEdit {
                    Layout.fillWidth: true
                    readOnly: true
                    selectByMouse: true
                    textFormat: TextEdit.PlainText
                    wrapMode: TextEdit.WrapAnywhere
                    font.family: Type.mono
                    font.pixelSize: Type.code.size
                    color: Theme.textPrimary
                    selectionColor: Theme.accent
                    selectedTextColor: Theme.textOnAccent
                    text: modelData.content
                    Accessible.role: Accessible.StaticText
                    Accessible.name: modelData.name
                    Accessible.description: text
                }
            }
        }
    }
    buttons: [
        Button {
            text: qsTr("Copy")
            enabled: dlg.sections.length > 0
            onClicked: {
                Platform.copy(dlg.sections.map(s => "## " + s.name + "\n" + s.content).join("\n\n"))
                dlg.toast(qsTr("Copied."))
            }
        },
        Item { Layout.fillWidth: true },
        Button {
            text: qsTr("Cancel")
            onClicked: dlg.close()
        },
        Button {
            variant: "primary"
            text: qsTr("Save")
            enabled: dlg.sections.length > 0
            onClicked: {
                saveDialog.selectedFile = "file://" + Platform.homePath + "/setu-diagnostics.txt"
                saveDialog.open()
            }
        }
    ]
}
