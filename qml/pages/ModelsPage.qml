import QtQuick
import QtQuick.Window
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

Page {
    id: page
    property var info: null
    property bool loading: true
    property string error

    readonly property var slotNames: ({ asr: qsTr("Hearing"), llm: qsTr("Brain"), tts: qsTr("Voice") })
    readonly property var icons: ({ asr: "ear", llm: "cpu", tts: "speech" })
    readonly property var statusNames: ({ missing: qsTr("Not downloaded"), unloaded: qsTr("Ready"), loading: qsTr("Loading"), loaded: qsTr("Running"), error: qsTr("Problem") })
    readonly property var slots: {
        const base = Daemon.models.length > 0 ? Daemon.models : (info ? info.slots : [])
        return ["llm", "tts", "asr"].map(id => base.find(s => s.slot === id)).filter(s => !!s)
    }
    readonly property var cloud: slots.filter(s => ["openai", "anthropic", "gemini", "elevenlabs"].includes(s.provider))
    readonly property var catalog: {
        let o = {}
        if (info)
            for (const c of info.catalog)
                o[c.id] = c
        return o
    }
    readonly property var names: {
        let n = {}
        if (info)
            for (const c of info.catalog)
                n[c.id] = c.name
        return n
    }

    function load() {
        Daemon.call("models.list", {}, (ok, r, e) => {
            page.loading = false
            page.error = ok ? "" : e.message
            if (!ok)
                return
            page.info = r
            downloads.seed(r.downloads)
        })
    }
    function describe(s) {
        if (s.provider !== "local")
            return { name: s.modelName || s.name, detail: s.provider }
        const main = s.slot === "asr" ? ((s.models || []).find(m => catalog[m] && catalog[m].role !== "streaming") || s.model) : s.model
        const m = catalog[main]
        if (!m)
            return { name: s.name || "", detail: "" }
        const v = (m.speakers || []).find(x => x.id === s.voice) || (m.speakers || [])[0]
        return { name: (m.family || m.name) + (m.params ? " " + m.params : ""), detail: v ? v.name + "  ·  " + v.accent : (m.quant || "") }
    }

    Component.onCompleted: {
        if (Daemon.connected)
            load()
        const p = Platform.overrides.preview || ""
        if (p.startsWith("library:"))
            Qt.callLater(() => library.show(p.slice(8)))
    }
    Connections {
        target: Daemon
        function onReconnected() { page.load() }
    }
    Subscription {
        topics: ["download.progress", "models.used"]
        onReceived: (topic, d) => {
            if (topic === "models.used" || d.status === "done" || d.status === "failed")
                refresh.restart()
        }
    }
    Timer {
        id: refresh
        interval: 500
        onTriggered: page.load()
    }

    ModelLibrary {
        id: library
        immediate: true
        onToast: text => page.toast(text)
        onPicked: page.load()
        onOnline: (slot, provider) => onlinePanel.edit(slot, page.slots.find(s => s.slot === slot) || {}, [], provider)
    }
    SlotPanel {
        id: onlinePanel
        onChanged: page.load()
    }
    ConfirmDialog {
        id: confirm
    }

    Banner {
        Layout.fillWidth: true
        visible: page.cloud.length > 0
        level: "warning"
        announce: false
        text: {
            const hosts = page.cloud.map(s => s.baseUrl).filter((v, i, a) => v && a.indexOf(v) === i)
            return qsTr("Some text is sent to %1.").arg(hosts.join(", "))
        }
    }

    LoadingState {
        Layout.fillWidth: true
        visible: page.loading
        what: page.title
    }
    ErrorState {
        Layout.fillWidth: true
        visible: !page.loading && page.error.length > 0
        text: page.error
        onAction: page.load()
    }

    GridLayout {
        Layout.fillWidth: true
        visible: page.slots.length > 0
        columns: width > Theme.px(720) ? 3 : 1
        columnSpacing: Theme.s3
        rowSpacing: Theme.s3
        Repeater {
            model: page.slots
            Card {
                required property var modelData
                readonly property var s: modelData
                readonly property var d: page.describe(modelData)
                Layout.fillWidth: true
                Layout.fillHeight: true
                highlighted: s.status === "loaded"
                Accessible.role: Accessible.Grouping
                Accessible.name: page.slotNames[s.slot]
                RowLayout {
                    spacing: Theme.s2
                    Icon {
                        name: page.icons[s.slot]
                        color: Theme.accent
                    }
                    Label {
                        Layout.fillWidth: true
                        text: page.slotNames[s.slot]
                        role: "label"
                        color: Theme.textSecondary
                    }
                }
                Label {
                    Layout.fillWidth: true
                    text: d.name
                    role: "heading"
                    elide: Text.ElideRight
                    wrapMode: Text.NoWrap
                }
                Label {
                    Layout.fillWidth: true
                    visible: text.length > 0
                    text: d.detail
                    role: "small"
                    color: Theme.textSecondary
                }
                StatusDot {
                    Layout.fillWidth: true
                    level: s.status === "loaded" ? "success" : s.status === "missing" || s.status === "error" ? "warning" : "info"
                    text: (page.statusNames[s.status] || "") + (s.device && s.device.length > 0 ? "  ·  " + s.device : "")
                }
                Item {
                    Layout.fillHeight: true
                }
                Button {
                    Layout.fillWidth: true
                    text: qsTr("Change")
                    Accessible.name: qsTr("Change %1").arg(page.slotNames[s.slot])
                    enabled: page.info !== null
                    onClicked: library.show(s.slot)
                }
            }
        }
    }

    Section {
        Layout.fillWidth: true
        visible: downloads.rows.length > 0
        title: qsTr("Downloads")
        DownloadList {
            id: downloads
            Layout.fillWidth: true
            names: page.names
        }
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("Performance")
        SettingSwitch {
            objectName: "setting:models.gpu"
            key: "models.gpu"
            enabled: page.info && page.info.gpu && page.info.gpu.available
            text: qsTr("Use the graphics card")
            description: page.info && page.info.gpu && page.info.gpu.available ? page.info.gpu.name + "  ·  " + Platform.formatBytes(page.info.gpu.totalBytes || 0) : qsTr("No graphics card found")
        }
        SettingSelect {
            objectName: "setting:models.unloadMinutes"
            key: "models.unloadMinutes"
            label: qsTr("Free memory when idle")
            options: [
                { value: 1, text: qsTr("After 1 minute") },
                { value: 5, text: qsTr("After 5 minutes") },
                { value: 15, text: qsTr("After 15 minutes") },
                { value: 0, text: qsTr("Never") }
            ]
        }
        Disclosure {
            Layout.fillWidth: true
            title: qsTr("Speed test")
            BenchmarkView {
                id: bench
                Layout.fillWidth: true
                items: page.info && page.info.benchmarks.length > 0 ? page.info.benchmarks[0].items.map(x => Object.assign({ status: "done" }, x)) : []
                onCompleted: page.load()
            }
        }
    }

    Section {
        Layout.fillWidth: true
        visible: page.info !== null
        title: qsTr("On this computer")
        description: page.info ? qsTr("%1 used  ·  %2 free").arg(Platform.formatBytes(page.info.diskUsedBytes)).arg(Platform.formatBytes(page.info.freeBytes)) : ""
        Repeater {
            model: page.info ? page.info.installed.filter(m => m.slot !== "codec") : []
            ListRow {
                required property var modelData
                Layout.fillWidth: true
                interactive: false
                title: modelData.name
                secondary: Platform.formatBytes(modelData.diskBytes) + (modelData.inUse ? "  ·  " + qsTr("In use") : "")
                iconName: "hard-drive"
                IconButton {
                    label: qsTr("Delete %1").arg(modelData.name)
                    iconName: "trash-2"
                    danger: true
                    onClicked: confirm.ask({
                        title: qsTr("Delete %1?").arg(modelData.name),
                        facts: [qsTr("Frees %1").arg(Platform.formatBytes(modelData.diskBytes))],
                        consequence: modelData.inUse ? qsTr("It's in use. Pick another one afterwards.") : qsTr("You can download it again later."),
                        actionLabel: qsTr("Delete"),
                        danger: true,
                        accept: () => Daemon.call("models.delete", { id: modelData.id }, (ok, r, e) => {
                            page.toast(ok ? qsTr("Freed %1.").arg(Platform.formatBytes(r.freedBytes)) : e.message)
                            page.load()
                        })
                    })
                }
            }
        }
        Button {
            text: qsTr("Browse all models")
            iconName: "package"
            onClicked: library.show("llm")
        }
    }
}
