import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

WizardStep {
    id: step
    key: "models"
    title: qsTr("Your setup")
    subtitle: qsTr("Picked for this computer. Change anything you like.")
    property var info: null
    property bool scanning: false
    readonly property var catalog: {
        let o = {}
        if (info)
            for (const c of info.catalog)
                o[c.id] = c
        return o
    }
    readonly property var slots: info ? info.slots : []
    function slotOf(id) {
        return slots.find(s => s.slot === id) || {}
    }
    readonly property var rows: ["llm", "tts", "asr"].map(id => {
        const s = slotOf(id)
        const local = s.provider === "local" || !s.provider
        const ids = local ? (s.models || []) : []
        const main = id === "asr" ? (ids.find(m => catalog[m] && catalog[m].role !== "streaming") || ids[0]) : ids[0]
        const m = catalog[main] || null
        const voice = m && m.speakers ? (m.speakers.find(v => v.id === s.voice) || m.speakers[0]) : null
        return {
            slot: id,
            local: local,
            name: !local ? (s.modelName || s.provider) : m ? (m.family || m.name) + (m.params ? " " + m.params : "") : (s.name || ""),
            detail: !local ? qsTr("Online") : voice ? voice.name + "  ·  " + voice.accent : m && m.quant ? m.quant : "",
            missing: local ? ids.reduce((a, x) => a + (catalog[x] ? catalog[x].missingBytes : 0), 0) : 0
        }
    })
    readonly property real total: rows.reduce((a, r) => a + r.missing, 0) + (info && catalog["speaker-campplus-en"] === undefined ? 0 : 0)
    readonly property var gpu: info ? info.gpu : null
    readonly property var titles: ({ llm: qsTr("Brain"), tts: qsTr("Voice"), asr: qsTr("Hearing") })
    readonly property var icons: ({ llm: "cpu", tts: "speech", asr: "ear" })
    summary: qsTr("This screen shows the models picked for this computer: the brain, the voice and hearing. The user can change any of them, or continue to download.")
    screenState: ({ brain: rows[0] ? rows[0].name : "", voice: rows[1] ? rows[1].name + " " + rows[1].detail : "", hearing: rows[2] ? rows[2].name : "", download: Platform.formatBytes(total), graphics: gpu && gpu.available ? gpu.name : "none" })
    primaryText: total > 0 ? qsTr("Download  ·  %1").arg(Platform.formatBytes(total)) : qsTr("Continue")
    primaryIcon: total > 0 ? "download" : "arrow-right"
    canContinue: info !== null && !scanning
    busy: scanning
    voiceChoices: [
        { id: "llm", label: qsTr("change the brain"), hint: "brain, language model, bigger, smaller, faster, smarter, online, Qwen, Llama, Gemma, Phi", phrases: ["change the brain", "another brain", "different model"] },
        { id: "tts", label: qsTr("change the voice"), hint: "voice, male, female, man, woman, accent, speaker, sound, Indian, Hindi, svara, Kokoro", phrases: ["change the voice", "another voice", "different voice"] },
        { id: "asr", label: qsTr("change hearing"), hint: "hearing, listening, speech recognition, Whisper, Parakeet", phrases: ["change hearing"] },
        { id: "next", label: qsTr("download"), hint: "download, start, continue", phrases: ["download", "start download", "looks good"] }
    ]
    function voiceChoice(id) {
        if (id === "next")
            advance()
        else
            library.show(id)
    }

    function load() {
        Daemon.call("models.list", {}, (ok, r, e) => {
            if (!ok) {
                step.toast(e.message)
                return
            }
            step.info = r
            const fresh = r.benchmarks.length > 0 && Date.now() - r.benchmarks[0].at < 7 * 86400000
            if (!fresh && !step.scanning) {
                step.scanning = true
                Daemon.call("benchmark.run", {}, (ok2) => {
                    if (!ok2)
                        step.scanning = false
                })
            }
        })
    }
    function commit() {
        if (info && !Settings.values["models.tier"])
            Settings.set("models.tier", info.recommended || info.tier || "balanced")
        return true
    }
    Component.onCompleted: load()
    Connections {
        target: Daemon
        function onReconnected() { step.load() }
    }
    Subscription {
        topics: ["benchmark.result", "models.state"]
        onReceived: (topic, d) => {
            if (topic === "benchmark.result")
                step.scanning = false
            reload.restart()
        }
    }
    Timer {
        id: reload
        interval: 250
        onTriggered: step.load()
    }

    ModelLibrary {
        id: library
        immediate: false
        onPicked: step.load()
        onToast: text => step.toast(text)
        onOnline: (slot, provider) => online.edit(slot, step.slotOf(slot), [], provider)
    }
    SlotPanel {
        id: online
        onChanged: step.load()
    }

    StepHeader {
        Layout.fillWidth: true
        title: step.title
        subtitle: step.subtitle
    }

    Card {
        Layout.fillWidth: true
        pad: Theme.s2
        Repeater {
            model: step.rows
            ListRow {
                required property var modelData
                required property int index
                Layout.fillWidth: true
                title: step.titles[modelData.slot]
                secondary: modelData.name + (modelData.detail.length > 0 ? "  ·  " + modelData.detail : "")
                iconName: step.icons[modelData.slot]
                iconColor: Theme.accent
                onClicked: library.show(modelData.slot)
                Button {
                    text: qsTr("Change")
                    Accessible.name: qsTr("Change %1").arg(step.titles[modelData.slot])
                    onClicked: library.show(modelData.slot)
                }
            }
        }
    }

    RowLayout {
        Layout.alignment: Qt.AlignHCenter
        spacing: Theme.s4
        StatusDot {
            showIcon: true
            level: step.gpu && step.gpu.available ? "success" : "info"
            text: step.scanning ? qsTr("Checking this computer…") : step.gpu && step.gpu.available ? qsTr("Graphics card: %1").arg(step.gpu.name) : qsTr("Runs on the processor")
        }
    }
}
