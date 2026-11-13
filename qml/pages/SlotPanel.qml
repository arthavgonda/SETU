import QtQuick
import QtQuick.Layouts
import QtQuick.Dialogs
import Setu.Core
import Setu.Theme
import Ledger

Dialog {
    id: panel
    property string slot: "llm"
    property var slotState: ({})
    property var installed: []
    property string provider: "local"
    property string localModel: ""
    property string filePath: ""
    property bool keySaved: false
    property bool busy: false
    property var test: null
    property var remoteModels: []
    signal changed

    readonly property var slotNames: ({ asr: qsTr("Speech recognition"), llm: qsTr("Language model"), tts: qsTr("Voice") })
    readonly property string keyRef: "api." + slot
    readonly property bool remote: provider !== "local"
    readonly property bool canSave: !remote && (localModel.length > 0 || filePath.length > 0)

    title: ({ llm: qsTr("Online brain"), tts: qsTr("Online voice"), asr: qsTr("Online hearing") })[slot] || ""

    function edit(slotId, current, installedModels, wanted) {
        slot = slotId
        slotState = current || {}
        installed = (installedModels || []).filter(m => m.slot === slotId)
        const p = slotState.provider || "local"
        provider = wanted ? wanted : p === "openai" && slotState.baseUrl && !/api\.openai\.com/.test(slotState.baseUrl) ? "custom" : p === "local" ? (slotId === "tts" ? "elevenlabs" : "openai") : p
        localModel = provider === "local" && slotState.model && !String(slotState.model).startsWith("/") ? slotState.model : ""
        filePath = provider === "local" && slotState.model && String(slotState.model).startsWith("/") ? slotState.model : ""
        test = null
        keySaved = false
        SecretStore.has(keyRef, found => panel.keySaved = found)
        open()
    }
    function save() {
        busy = true
        const done = (ok, r, e) => {
            panel.busy = false
            if (ok) {
                panel.changed()
                panel.close()
            } else {
                panel.test = { ok: false, message: e.message }
            }
        }
        Daemon.call("models.set", filePath.length > 0 ? { slot: slot, provider: "local", path: filePath } : { slot: slot, provider: "local", model: localModel }, done)
    }

    FileDialog {
        id: fileDialog
        title: panel.slot === "tts" ? qsTr("Choose a voice file") : qsTr("Choose a language model file")
        nameFilters: panel.slot === "tts" ? [qsTr("Piper voices (*.onnx)")] : [qsTr("GGUF models (*.gguf)")]
        currentFolder: "file://" + Platform.homePath
        onAccepted: {
            panel.filePath = Platform.localFile(selectedFile)
            panel.localModel = ""
        }
    }

    Segmented {
        Layout.fillWidth: true
        label: qsTr("Provider")
        value: panel.provider
        options: panel.slot === "llm" ? [
            { value: "openai", text: qsTr("OpenAI") },
            { value: "anthropic", text: qsTr("Anthropic") },
            { value: "gemini", text: qsTr("Gemini") },
            { value: "ollama", text: qsTr("Ollama") },
            { value: "lmstudio", text: qsTr("LM Studio") },
            { value: "custom", text: qsTr("Other API") }
        ] : panel.slot === "tts" ? [
            { value: "elevenlabs", text: qsTr("ElevenLabs") },
            { value: "openai", text: qsTr("OpenAI") },
            { value: "custom", text: qsTr("Other API") }
        ] : [
            { value: "openai", text: qsTr("OpenAI") },
            { value: "custom", text: qsTr("Other API") }
        ]
        onActivated: v => {
            panel.provider = v
            panel.test = null
        }
    }

    ColumnLayout {
        Layout.fillWidth: true
        visible: !panel.remote
        spacing: Theme.s3
        Select {
            Layout.fillWidth: true
            label: qsTr("Downloaded model")
            options: panel.installed.map(m => ({ value: m.id, text: m.name + "  ·  " + Platform.formatBytes(m.diskBytes) }))
            value: panel.localModel
            emptyText: panel.filePath.length > 0 ? qsTr("Using a file") : qsTr("Choose a model")
            onActivated: v => {
                panel.localModel = v
                panel.filePath = ""
            }
        }
        RowLayout {
            Layout.fillWidth: true
            visible: panel.slot !== "asr"
            spacing: Theme.s2
            Label {
                Layout.fillWidth: true
                text: panel.filePath.length > 0 ? panel.filePath : panel.slot === "tts" ? qsTr("Or use a Piper voice file from this computer.") : qsTr("Or use a .gguf file from this computer.")
                role: panel.filePath.length > 0 ? "mono" : "body"
                color: panel.filePath.length > 0 ? Theme.textPrimary : Theme.textSecondary
                wrapMode: Text.WrapAnywhere
            }
            Button {
                text: qsTr("Add a file")
                iconName: "folder-open"
                onClicked: fileDialog.open()
            }
        }
    }

    ProviderForm {
        id: form
        Layout.fillWidth: true
        visible: panel.remote
        slot: panel.slot
        provider: panel.provider === "custom" ? "openai" : (panel.remote ? panel.provider : "openai")
        custom: panel.provider === "custom"
        current: panel.slotState
        onChanged: {
            panel.changed()
            panel.close()
        }
    }
    Button {
        visible: panel.remote && panel.keySaved
        variant: "quiet"
        text: qsTr("Remove saved key")
        onClicked: SecretStore.remove(panel.keyRef, ok => {
            panel.keySaved = !ok
            form.keySaved = !ok
        })
    }

    Label {
        id: testLine
        Layout.fillWidth: true
        visible: panel.test !== null
        text: panel.test ? (panel.test.ok ? qsTr("Connected in %1 ms. %2").arg(panel.test.latencyMs).arg(panel.test.message || "") : panel.test.message) : ""
        color: panel.test && panel.test.ok ? Theme.success : Theme.danger
    }

    buttons: [
        Item { Layout.fillWidth: true },
        Button {
            text: panel.remote ? qsTr("Close") : qsTr("Cancel")
            onClicked: panel.close()
        },
        Button {
            visible: !panel.remote
            variant: "primary"
            text: qsTr("Use this")
            enabled: panel.canSave && !panel.busy
            onClicked: panel.save()
        }
    ]
}
