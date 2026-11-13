import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

WizardStep {
    id: step
    key: "setup"
    title: phase === "ready" ? qsTr("All set up") : phase === "warming" ? qsTr("Getting ready") : qsTr("Downloading")
    subtitle: phase === "ready" ? "" : phase === "warming" ? qsTr("Loading everything into memory.") : qsTr("You can keep using your computer.")
    summary: qsTr("Models are downloading and being set up. The user waits here until everything is ready.")
    property var info: null
    property var rows: ({})
    property string phase: "starting"
    property string problem: ""
    property var disk: null
    property var devices: null
    readonly property var catalog: {
        let o = {}
        if (info)
            for (const c of info.catalog)
                o[c.id] = c
        return o
    }
    readonly property var wanted: {
        if (!info)
            return []
        let ids = []
        for (const s of info.slots)
            if (s.provider === "local" && s.status === "missing")
                for (const id of s.models || [])
                    for (const x of ((catalog[id] || {}).requires || []).concat([id]))
                        if (!ids.includes(x) && !(catalog[x] && catalog[x].installed))
                            ids.push(x)
        return ids
    }
    readonly property var list: Object.keys(rows).map(k => rows[k])
    readonly property real bytes: list.reduce((a, r) => a + (r.status === "done" ? r.total : r.bytes || 0), 0)
    readonly property real totalBytes: list.reduce((a, r) => a + (r.total || 0), 0)
    readonly property real fraction: totalBytes > 0 ? Math.min(1, bytes / totalBytes) : (phase === "ready" ? 1 : 0)
    readonly property bool failed: list.some(r => r.status === "failed")
    canContinue: phase === "ready"
    showPrimary: phase === "ready"
    busy: phase !== "ready" && phase !== "failed"
    screenState: ({ phase: phase, percent: Math.round(fraction * 100) })

    function load(then) {
        Daemon.call("models.list", {}, (ok, r, e) => {
            if (!ok) {
                step.problem = e.message
                return
            }
            step.info = r
            let rows = Object.assign({}, step.rows)
            for (const d of r.downloads)
                rows[d.id] = d
            step.rows = rows
            if (then)
                then()
        })
    }
    function start() {
        problem = ""
        disk = null
        if (wanted.length === 0) {
            warm()
            return
        }
        phase = "downloading"
        let rows = Object.assign({}, step.rows)
        for (const id of wanted)
            if (!rows[id])
                rows[id] = { id: id, name: catalog[id] ? catalog[id].name : id, bytes: 0, total: catalog[id] ? catalog[id].sizeBytes : 0, status: "queued" }
        step.rows = rows
        Daemon.call("models.download", { ids: wanted.concat(["speaker-campplus-en"]) }, (ok, r, e) => {
            if (ok)
                return
            if (e.code === "disk_full")
                step.disk = e.details || {}
            step.problem = e.message
            step.phase = "failed"
        })
    }
    function check() {
        load(() => {
            const missing = step.info.slots.some(s => s.provider === "local" && s.status === "missing")
            if (!missing && step.phase !== "warming" && step.phase !== "ready")
                step.warm()
        })
    }
    function warm() {
        phase = "warming"
        Daemon.call("models.warm", {}, (ok, r, e) => {
            if (!ok) {
                step.problem = e.message
                step.phase = "failed"
                return
            }
            step.devices = r
            step.phase = "ready"
            Announcer.announce(step, qsTr("Everything is ready."), false)
            step.say(qsTr("Namaste! I'm all set up. Let's get to know each other."))
            autoNext.start()
        })
    }
    Component.onCompleted: load(start)
    Timer {
        id: autoNext
        interval: Motion.reduced ? 2500 : 4200
        onTriggered: step.advance()
    }
    Subscription {
        topics: ["download.progress"]
        onReceived: (topic, d) => {
            let rows = Object.assign({}, step.rows)
            rows[d.id] = d
            step.rows = rows
            if (d.status === "done")
                checkLater.restart()
            if (d.status === "failed") {
                step.problem = d.message || qsTr("A download failed.")
                step.phase = "failed"
            }
        }
    }
    Timer {
        id: checkLater
        interval: 400
        onTriggered: step.check()
    }

    StepHeader {
        Layout.fillWidth: true
        title: step.title
        subtitle: step.subtitle
    }

    Item {
        Layout.alignment: Qt.AlignHCenter
        Layout.preferredWidth: Theme.px(190)
        Layout.preferredHeight: Theme.px(190)
        LottieAnimation {
            anchors.fill: parent
            visible: step.phase !== "ready"
            source: visible ? "qrc:/lottie/download.json" : ""
            playing: !Motion.reduced
            stillProgress: 0.5
            Accessible.ignored: true
        }
        Rectangle {
            anchors.centerIn: parent
            visible: step.phase === "ready"
            width: Theme.px(120)
            height: width
            radius: width / 2
            color: Theme.success
            scale: visible ? 1 : 0.4
            Behavior on scale {
                NumberAnimation { duration: Motion.morph; easing.type: Easing.OutBack; easing.overshoot: Motion.overshoot * 2 }
            }
            Icon {
                anchors.centerIn: parent
                name: "check"
                size: Theme.px(56)
                color: Theme.textOnAccent
            }
        }
    }

    ColumnLayout {
        Layout.fillWidth: true
        visible: step.phase === "downloading" || step.phase === "starting"
        spacing: Theme.s2
        Label {
            Layout.alignment: Qt.AlignHCenter
            text: Math.round(step.fraction * 100) + "%"
            role: "display"
            color: Theme.accent
        }
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 8
            radius: 4
            color: Theme.bgSunken
            Accessible.role: Accessible.ProgressBar
            Accessible.name: qsTr("Download progress")
            Accessible.description: Math.round(step.fraction * 100) + "%"
            Rectangle {
                width: parent.width * step.fraction
                height: parent.height
                radius: 4
                color: Theme.accent
                Behavior on width {
                    NumberAnimation { duration: 400; easing.type: Easing.OutCubic }
                }
            }
        }
        Label {
            Layout.alignment: Qt.AlignHCenter
            role: "small"
            color: Theme.textSecondary
            text: step.totalBytes > 0 ? qsTr("%1 of %2").arg(Platform.formatBytes(step.bytes)).arg(Platform.formatBytes(step.totalBytes)) : ""
        }
    }

    Card {
        Layout.fillWidth: true
        visible: step.list.length > 0 && step.phase !== "ready"
        pad: Theme.s3
        Repeater {
            model: step.list
            RowLayout {
                required property var modelData
                Layout.fillWidth: true
                spacing: Theme.s3
                Icon {
                    name: modelData.status === "done" ? "circle-check" : modelData.status === "failed" ? "circle-x" : "download"
                    color: modelData.status === "done" ? Theme.success : modelData.status === "failed" ? Theme.danger : Theme.textSecondary
                    size: Theme.px(18)
                }
                Label {
                    Layout.fillWidth: true
                    text: modelData.name || modelData.id
                    elide: Text.ElideRight
                    wrapMode: Text.NoWrap
                }
                Rectangle {
                    Layout.preferredWidth: Theme.px(120)
                    Layout.preferredHeight: 4
                    radius: 2
                    color: Theme.bgSunken
                    visible: modelData.status !== "done"
                    Rectangle {
                        width: parent.width * (modelData.total > 0 ? Math.min(1, (modelData.bytes || 0) / modelData.total) : 0)
                        height: parent.height
                        radius: 2
                        color: Theme.accent
                    }
                }
                Label {
                    role: "small"
                    color: Theme.textSecondary
                    text: modelData.status === "verifying" ? qsTr("Checking") : modelData.status === "extracting" ? qsTr("Unpacking") : modelData.status === "waiting_network" ? qsTr("Offline") : modelData.status === "done" ? "" : Platform.formatBytes(modelData.total || 0)
                }
            }
        }
    }

    Card {
        Layout.fillWidth: true
        visible: step.phase === "ready" && step.devices !== null
        pad: Theme.s3
        Repeater {
            model: [{ k: "llm", t: qsTr("Brain") }, { k: "tts", t: qsTr("Voice") }, { k: "asr", t: qsTr("Hearing") }]
            RowLayout {
                required property var modelData
                Layout.fillWidth: true
                spacing: Theme.s3
                Icon {
                    name: "circle-check"
                    color: Theme.success
                    size: Theme.px(18)
                }
                Label {
                    Layout.fillWidth: true
                    text: modelData.t
                }
                Label {
                    role: "small"
                    color: Theme.textSecondary
                    text: step.devices ? (step.devices[modelData.k] || qsTr("Online")) : ""
                }
            }
        }
    }

    Banner {
        Layout.fillWidth: true
        visible: step.phase === "failed"
        level: "danger"
        text: step.problem
        actionText: qsTr("Try again")
        secondaryText: step.disk ? qsTr("Free up space") : ""
        onAction: step.start()
        onSecondary: Platform.openPath(Daemon.paths.models || Platform.homePath)
    }
}
