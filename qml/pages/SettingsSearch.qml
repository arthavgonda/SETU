import QtQuick
import QtQuick.Templates as T
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

ColumnLayout {
    id: root
    property var pages: []
    property bool compact: false
    property alias field: input.field
    signal chosen(string page, string key)
    spacing: 0

    readonly property var index: [
        { page: "voice", key: "wake.test", title: qsTr("Test wake word") },
        { page: "voice", key: "wake.sensitivity", title: qsTr("Wake sensitivity") },
        { page: "voice", key: "audio.inputDevice", title: qsTr("Input device") },
        { page: "voice", key: "audio.noiseReduction", title: qsTr("Noise reduction") },
        { page: "voice", key: "voice.stopWord", title: qsTr("Stop word") },
        { page: "voice", key: "audio.outputDevice", title: qsTr("Output device") },
        { page: "voice", key: "tts.voice", title: qsTr("Voice") },
        { page: "voice", key: "tts.rate", title: qsTr("Speech rate") },
        { page: "voice", key: "tts.volume", title: qsTr("Volume") },
        { page: "voice", key: "tts.speakReplies", title: qsTr("Speak replies") },
        { page: "voice", key: "audio.listeningSound", title: qsTr("Play listening sound") },
        { page: "voice", key: "tts.readConfirmations", title: qsTr("Read confirmations aloud") },
        { page: "voice", key: "session.activeMinutes", title: qsTr("Active window length") },
        { page: "voice", key: "session.graceSeconds", title: qsTr("Flow grace period") },
        { page: "voice", key: "session.endSilence", title: qsTr("End-of-speech silence") },
        { page: "voice", key: "audio.quietVoice", title: qsTr("Quiet-voice mode") },
        { page: "voice", key: "speaker.verification", title: qsTr("Speaker verification") },
        { page: "privacy", key: "face.unlock", title: qsTr("Face unlock") },
        { page: "models", key: "models.tier", title: qsTr("Model tier") },
        { page: "models", key: "models.unloadMinutes", title: qsTr("Unload models when idle") },
        { page: "models", key: "benchmark", title: qsTr("Run benchmark again") },
        { page: "permissions", key: "permissions.reset", title: qsTr("Reset all permissions") },
        { page: "plugins", key: "plugins.developerMode", title: qsTr("Developer mode") },
        { page: "privacy", key: "privacy.learn", title: qsTr("Learn from my commands") },
        { page: "privacy", key: "history.keep", title: qsTr("Keep history") },
        { page: "privacy", key: "history.retention", title: qsTr("History retention") },
        { page: "privacy", key: "privacy.crashReports", title: qsTr("Anonymous reports") },
        { page: "privacy", key: "data.wipe", title: qsTr("Delete everything and reset") },
        { page: "accessibility", key: "ui.textScale", title: qsTr("Text size") },
        { page: "accessibility", key: "ui.theme", title: qsTr("Theme") },
        { page: "accessibility", key: "ui.readingFont", title: qsTr("Reading font") },
        { page: "accessibility", key: "ui.reduceMotion", title: qsTr("Reduce motion") },
        { page: "accessibility", key: "ui.largeCaptions", title: qsTr("Larger captions") },
        { page: "accessibility", key: "confirm.method", title: qsTr("Confirmation method") },
        { page: "accessibility", key: "ui.timeoutScale", title: qsTr("Timeout lengthening") },
        { page: "accessibility", key: "ui.announceDetail", title: qsTr("Screen reader announcements") },
        { page: "accessibility", key: "ui.switchScanning", title: qsTr("Switch scanning") },
        { page: "accessibility", key: "ui.notch", title: qsTr("Show the notch") },
        { page: "accessibility", key: "media.lyrics", title: qsTr("Show song lyrics") },
        { page: "general", key: "ui.language", title: qsTr("Language") },
        { page: "general", key: "startup.atLogin", title: qsTr("Start when I sign in") },
        { page: "general", key: "general.switchInput", title: qsTr("Switch input") },
        { page: "general", key: "shortcuts", title: qsTr("Keyboard shortcuts") },
        { page: "general", key: "update.channel", title: qsTr("Update channel") },
        { page: "general", key: "diagnostics", title: qsTr("Export diagnostics") },
        { page: "about", key: "licenses", title: qsTr("Third-party licenses") }
    ]

    readonly property var results: {
        const q = input.text.trim().toLowerCase()
        if (q.length < 2)
            return []
        let out = []
        for (const p of pages)
            if (p.title.toLowerCase().includes(q))
                out.push({ page: p.id, key: "", title: p.title, where: qsTr("Page") })
        for (const s of index)
            if (s.title.toLowerCase().includes(q))
                out.push({ page: s.page, key: s.key, title: s.title, where: (pages.find(p => p.id === s.page) || {}).title })
        return out.slice(0, 8)
    }

    SearchField {
        id: input
        Layout.fillWidth: true
        label: root.compact ? "" : qsTr("Find a setting")
        placeholder: root.compact ? qsTr("Find a setting") : ""
        field.objectName: "settings-search"
        field.Accessible.name: qsTr("Find a setting")
        field.Keys.onDownPressed: if (results.count > 0) results.forceActiveFocus()
        onAccepted: if (root.results.length > 0) {
            root.chosen(root.results[0].page, root.results[0].key)
            input.text = ""
        }
    }
    ListView {
        id: results
        Layout.fillWidth: true
        Layout.preferredHeight: contentHeight
        visible: count > 0
        model: root.results
        interactive: false
        keyNavigationEnabled: true
        Accessible.role: Accessible.List
        Accessible.name: qsTr("Search results")
        Keys.onUpPressed: event => {
            if (currentIndex === 0)
                input.forceActiveFocus()
            else
                event.accepted = false
        }
        delegate: ListRow {
            required property var modelData
            width: results.width
            title: modelData.title
            secondary: modelData.where
            onClicked: {
                root.chosen(modelData.page, modelData.key)
                input.text = ""
            }
        }
    }
}
