import QtQuick
import QtQuick.Layouts
import QtQuick.Dialogs
import Setu.Core
import Setu.Theme
import Ledger

Dialog {
    id: lib
    property string slot: "llm"
    property bool immediate: true
    property var info: null
    property string query: ""
    property var progress: ({})
    property var picks: ({})
    property bool fresh: true
    signal picked(string id, string voice)
    signal online(string slot, string provider)
    signal toast(string text)

    readonly property var slotTitles: ({ llm: qsTr("Brain"), tts: qsTr("Voice"), asr: qsTr("Hearing") })
    readonly property var current: info ? (info.slots.find(s => s.slot === slot) || {}) : {}
    readonly property string currentId: current.provider === "local" ? (slot === "asr" ? ((current.models || []).find(id => { const m = catalogById[id]; return m && m.role !== "streaming" }) || "") : (current.model || "")) : ""
    readonly property string currentVoice: current.voice || ""
    readonly property var catalogById: {
        let o = {}
        if (info)
            for (const c of info.catalog)
                o[c.id] = c
        return o
    }
    readonly property string recommendedId: {
        if (!info)
            return ""
        const t = info.tiers.find(x => x.id === (info.recommended || info.tier || "balanced"))
        if (!t)
            return ""
        return slot === "llm" ? t.models.llm : slot === "tts" ? t.models.tts : (t.models.asr.find(id => catalogById[id] && catalogById[id].role !== "streaming") || "")
    }
    readonly property bool gpu: info && info.gpu ? info.gpu.available === true : false
    readonly property var families: {
        if (!info)
            return []
        const q = query.trim().toLowerCase()
        let out = []
        let seen = {}
        for (const m of info.catalog) {
            if (m.slot !== slot || (slot === "asr" && m.role === "streaming"))
                continue
            const f = m.family || m.name
            if (!seen[f]) {
                seen[f] = { name: f, description: m.description || "", items: [], speakers: m.speakers || [], gpu: m.gpu || "" }
                out.push(seen[f])
            }
            seen[f].items.push(m)
        }
        if (q.length > 0)
            out = out.filter(f => f.name.toLowerCase().includes(q) || f.items.some(i => (i.params || "").toLowerCase().includes(q) || i.name.toLowerCase().includes(q)) || f.speakers.some(s => s.name.toLowerCase().includes(q) || s.accent.toLowerCase().includes(q)))
        const rank = f => f.items.some(i => i.id === currentId) ? 0 : f.items.some(i => i.id === recommendedId) ? 1 : 2
        out.sort((a, b) => rank(a) - rank(b) || Math.min(...a.items.map(i => i.sizeBytes)) - Math.min(...b.items.map(i => i.sizeBytes)))
        return out
    }

    title: slotTitles[slot] || ""
    preferredWidth: Theme.px(760)

    function show(slotId) {
        slot = slotId
        query = ""
        picks = {}
        fresh = true
        settle.restart()
        load()
        open()
    }
    Timer {
        id: settle
        interval: 900
        onTriggered: lib.fresh = false
    }
    onSlotChanged: {
        fresh = true
        settle.restart()
    }
    function load() {
        Daemon.call("models.list", {}, (ok, r, e) => {
            if (ok) {
                lib.info = r
                let p = {}
                for (const d of r.downloads)
                    p[d.id] = d
                lib.progress = p
            } else {
                lib.toast(e.message)
            }
        })
    }
    function busyFor(item) {
        const ids = [item.id].concat(item.requires || [])
        return ids.map(id => progress[id]).filter(d => d && ["queued", "downloading", "verifying", "extracting", "waiting_network", "paused"].includes(d.status))
    }
    function choose(item, voice) {
        if (!immediate) {
            Daemon.call("models.set", { slot: slot, provider: "local", model: item.id, voice: voice }, (ok, r, e) => {
                if (!ok) {
                    lib.toast(e.message)
                    return
                }
                lib.picked(item.id, voice)
                lib.close()
            })
            return
        }
        Daemon.call("models.use", { id: item.id, voice: voice }, (ok, r, e) => {
            if (!ok) {
                lib.toast(e.message)
                return
            }
            if (r.queued && r.queued.length > 0)
                Announcer.announce(lib.contentItem, qsTr("Downloading %1. It switches over by itself when ready.").arg(item.name), false)
            lib.picked(item.id, voice)
            lib.load()
        })
    }

    Subscription {
        topics: ["download.progress", "models.used", "models.state"]
        active: lib.opened
        onReceived: (topic, d) => {
            if (topic === "download.progress") {
                let p = Object.assign({}, lib.progress)
                p[d.id] = d
                lib.progress = p
                if (d.status === "done" || d.status === "failed")
                    reload.restart()
            } else {
                reload.restart()
            }
        }
    }
    Timer {
        id: reload
        interval: 300
        onTriggered: lib.load()
    }
    FileDialog {
        id: fileDialog
        title: lib.slot === "tts" ? qsTr("Choose a voice file") : qsTr("Choose a model file")
        nameFilters: lib.slot === "tts" ? [qsTr("Piper voices (*.onnx)")] : [qsTr("GGUF models (*.gguf)")]
        currentFolder: "file://" + Platform.homePath
        onAccepted: Daemon.call("models.set", { slot: lib.slot, provider: "local", path: Platform.localFile(selectedFile) }, (ok, r, e) => {
            if (!ok) {
                lib.toast(e.message)
                return
            }
            lib.picked("", "")
            lib.load()
        })
    }

    RowLayout {
        Layout.fillWidth: true
        spacing: Theme.s2
        Repeater {
            model: [{ id: "llm", text: qsTr("Brain"), icon: "cpu" }, { id: "tts", text: qsTr("Voice"), icon: "speech" }, { id: "asr", text: qsTr("Hearing"), icon: "ear" }]
            Pill {
                required property var modelData
                text: modelData.text
                iconName: modelData.icon
                selected: lib.slot === modelData.id
                onClicked: lib.slot = modelData.id
            }
        }
        Item {
            Layout.fillWidth: true
        }
        StatusDot {
            visible: lib.gpu
            level: "success"
            text: lib.info && lib.info.gpu && lib.info.gpu.name ? lib.info.gpu.name : ""
        }
    }

    SearchField {
        id: search
        Layout.fillWidth: true
        label: ""
        placeholder: qsTr("Search models")
        onTextChanged: lib.query = text
    }

    LoadingState {
        Layout.fillWidth: true
        visible: lib.info === null
        what: lib.title
    }

    Repeater {
        model: lib.families
        Card {
            id: fam
            required property var modelData
            required property int index
            readonly property var items: modelData.items
            readonly property var sizes: {
                let out = []
                for (const i of items.slice().sort((a, b) => a.sizeBytes - b.sizeBytes))
                    if (!out.includes(i.params || ""))
                        out.push(i.params || "")
                return out
            }
            readonly property var initial: items.find(i => i.id === lib.currentId) || items.find(i => i.id === lib.recommendedId) || items.filter(i => i.fits !== false && !i.slow).sort((a, b) => b.sizeBytes - a.sizeBytes).find(i => !i.quant || i.quant === "Q4" || i.quant === "Q8" && i.kind === "kokoro") || items[0]
            property string size: ""
            property string quant: ""
            property bool touched: false
            readonly property var quants: items.filter(i => (i.params || "") === size).map(i => i.quant || "").filter(q => q.length > 0)
            readonly property var item: items.find(i => (i.params || "") === size && (i.quant || "") === quant) || items.find(i => (i.params || "") === size) || items[0]
            readonly property var voices: {
                const lang = String(Settings.values["ui.language"] || "en")
                let v = modelData.speakers.slice()
                v.sort((a, b) => (b.accent === "Indian English") - (a.accent === "Indian English") || (b.languages.includes(lang)) - (a.languages.includes(lang)))
                return v
            }
            property string voice: ""
            readonly property bool inUse: item && item.id === lib.currentId && (voices.length === 0 || voice === lib.currentVoice || lib.currentVoice.length === 0 && voice === voices[0].id)
            readonly property var busy: item ? lib.busyFor(item) : []
            readonly property bool recommended: items.some(i => i.id === lib.recommendedId)
            readonly property bool needsGpu: modelData.gpu === "needed"
            Layout.fillWidth: true
            highlighted: inUse
            opacity: 0
            transform: Translate {
                id: lift
                y: 18
            }
            function remember() {
                touched = true
                let p = lib.picks
                p[modelData.name] = { size: size, quant: quant, voice: voice }
            }
            function defaults() {
                if (touched)
                    return
                const it = initial
                size = it ? (it.params || "") : ""
                quant = it ? (it.quant || "") : ""
                const inUse = it && it.id === lib.currentId
                voice = inUse && lib.currentVoice.length > 0 ? lib.currentVoice : (voices.length > 0 ? voices[0].id : "")
            }
            onInitialChanged: defaults()
            Connections {
                target: lib
                function onCurrentVoiceChanged() { fam.defaults() }
                function onCurrentIdChanged() { fam.defaults() }
            }
            Component.onCompleted: {
                const p = lib.picks[modelData.name]
                if (p) {
                    size = p.size
                    quant = p.quant
                    voice = p.voice
                    touched = true
                } else {
                    defaults()
                }
                if (lib.fresh) {
                    appear.start()
                } else {
                    opacity = 1
                    lift.y = 0
                }
            }
            ParallelAnimation {
                id: appear
                PauseAnimation { duration: Math.min(fam.index, 8) * Motion.stagger }
                NumberAnimation { target: fam; property: "opacity"; to: 1; duration: Motion.spring; easing.type: Easing.OutCubic }
                NumberAnimation { target: lift; property: "y"; to: 0; duration: Motion.spring; easing.type: Easing.OutBack; easing.overshoot: Motion.overshoot }
            }
            Accessible.role: Accessible.Grouping
            Accessible.name: modelData.name

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.s3
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 2
                    RowLayout {
                        spacing: Theme.s2
                        Label {
                            text: fam.modelData.name
                            role: "heading"
                        }
                        Label {
                            visible: fam.recommended
                            text: qsTr("Recommended")
                            role: "small"
                            color: Theme.accent
                            font.weight: Font.DemiBold
                        }
                    }
                    Label {
                        Layout.fillWidth: true
                        visible: text.length > 0
                        text: fam.item && fam.item.description ? fam.item.description : fam.modelData.description
                        role: "small"
                        color: Theme.textSecondary
                    }
                }
                Icon {
                    visible: fam.inUse
                    name: "circle-check"
                    color: Theme.success
                }
            }

            Flow {
                Layout.fillWidth: true
                visible: fam.sizes.length > 1
                spacing: Theme.s1
                Repeater {
                    model: fam.sizes
                    Pill {
                        required property string modelData
                        text: modelData
                        selected: fam.size === modelData
                        onClicked: {
                            fam.size = modelData
                            if (!fam.quants.includes(fam.quant))
                                fam.quant = fam.quants[0] || ""
                            fam.remember()
                        }
                    }
                }
            }
            Flow {
                Layout.fillWidth: true
                visible: fam.quants.length > 1
                spacing: Theme.s1
                Repeater {
                    model: fam.quants
                    Pill {
                        required property string modelData
                        readonly property var it: fam.items.find(i => (i.params || "") === fam.size && i.quant === modelData)
                        text: modelData
                        detail: it ? Platform.formatBytes(it.sizeBytes) : ""
                        selected: fam.quant === modelData
                        onClicked: {
                            fam.quant = modelData
                            fam.remember()
                        }
                    }
                }
            }
            Flow {
                Layout.fillWidth: true
                visible: fam.voices.length > 0
                spacing: Theme.s1
                Repeater {
                    model: fam.voices.slice(0, fam.modelData.name === "Svara" ? 8 : 12)
                    Pill {
                        required property var modelData
                        text: modelData.name
                        detail: modelData.accent + (modelData.gender === "female" ? " ♀" : " ♂")
                        selected: fam.voice === modelData.id
                        onClicked: {
                            fam.voice = modelData.id
                            fam.remember()
                        }
                    }
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.s3
                Label {
                    Layout.fillWidth: true
                    role: "small"
                    color: fam.item && fam.item.fits === false ? Theme.danger : Theme.textSecondary
                    text: {
                        const it = fam.item
                        if (!it)
                            return ""
                        if (it.fits === false)
                            return fam.needsGpu ? qsTr("Needs a graphics card") : qsTr("Too big for this computer")
                        let parts = [qsTr("%1 memory").arg(Platform.formatBytes(it.ramBytes))]
                        if (lib.gpu && (it.gpu === "faster" || it.gpu === "needed"))
                            parts.push(qsTr("Runs on your graphics card"))
                        if (it.slow)
                            parts.push(qsTr("May be slow"))
                        return parts.join("  ·  ")
                    }
                }
                Progress {
                    Layout.preferredWidth: Theme.px(200)
                    visible: fam.busy.length > 0
                    label: ""
                    bytes: fam.busy.reduce((a, d) => a + (d.bytes || 0), 0)
                    total: fam.busy.reduce((a, d) => a + (d.total || 0), 0)
                    status: fam.busy.length > 0 ? fam.busy[0].status : ""
                }
                Button {
                    visible: fam.busy.length === 0
                    variant: fam.inUse ? "quiet" : (fam.item && fam.item.installed ? "secondary" : "primary")
                    large: false
                    enabled: !fam.inUse && fam.item && fam.item.fits !== false
                    iconName: fam.inUse ? "check" : (fam.item && !fam.item.installed && lib.immediate ? "download" : "")
                    text: fam.inUse ? qsTr("In use") : !lib.immediate ? qsTr("Choose") : fam.item && fam.item.installed ? qsTr("Use") : qsTr("Get · %1").arg(Platform.formatBytes(fam.item ? fam.item.missingBytes : 0))
                    Accessible.name: text + " " + fam.modelData.name + " " + fam.size + " " + fam.quant
                    onClicked: lib.choose(fam.item, fam.voice)
                }
            }
        }
    }

    RowLayout {
        Layout.fillWidth: true
        Layout.topMargin: Theme.s2
        spacing: Theme.s2
        Label {
            text: qsTr("Online")
            role: "label"
            color: Theme.textSecondary
        }
        Flow {
            Layout.fillWidth: true
            spacing: Theme.s1
            Repeater {
                model: lib.slot === "llm" ? [
                    { id: "openai", text: qsTr("OpenAI") }, { id: "anthropic", text: qsTr("Claude") }, { id: "gemini", text: qsTr("Gemini") },
                    { id: "ollama", text: qsTr("Ollama") }, { id: "lmstudio", text: qsTr("LM Studio") }, { id: "custom", text: qsTr("Other") }
                ] : lib.slot === "tts" ? [{ id: "elevenlabs", text: qsTr("ElevenLabs") }, { id: "openai", text: qsTr("OpenAI") }, { id: "custom", text: qsTr("Other") }]
                  : [{ id: "openai", text: qsTr("OpenAI") }, { id: "custom", text: qsTr("Other") }]
                Pill {
                    required property var modelData
                    text: modelData.text
                    selected: lib.current.provider === modelData.id
                    onClicked: {
                        lib.online(lib.slot, modelData.id)
                        lib.close()
                    }
                }
            }
        }
        Button {
            visible: lib.slot !== "asr"
            variant: "quiet"
            iconName: "folder-open"
            text: qsTr("Use a file")
            onClicked: fileDialog.open()
        }
    }

    buttons: [
        Item { Layout.fillWidth: true },
        Button {
            text: qsTr("Done")
            onClicked: lib.close()
        }
    ]
}
