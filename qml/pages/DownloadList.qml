import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

ColumnLayout {
    id: root
    property var rows: []
    property var names: ({})
    readonly property bool active: rows.some(r => ["queued", "downloading", "paused", "waiting_network", "verifying", "extracting"].indexOf(r.status) >= 0)
    readonly property bool failed: rows.some(r => r.status === "failed")
    signal finished(string id)
    spacing: Theme.s3

    function seed(list) {
        let out = rows.slice()
        for (const d of list || []) {
            const i = out.findIndex(r => r.id === d.id)
            if (i >= 0)
                out[i] = d
            else
                out.push(d)
        }
        rows = out
    }
    function statusText(r) {
        const pct = r.total > 0 ? Math.floor(100 * r.bytes / r.total) : 0
        switch (r.status) {
        case "queued": return qsTr("Waiting")
        case "downloading": return ""
        case "paused": return qsTr("Paused at %1%.").arg(pct)
        case "waiting_network": return qsTr("Paused at %1%. The connection dropped. It will resume when you're online.").arg(pct)
        case "verifying": return qsTr("Checking the download")
        case "extracting": return qsTr("Unpacking")
        case "done": return qsTr("Downloaded and verified")
        case "failed": return r.message || qsTr("The download failed.")
        case "cancelled": return qsTr("Cancelled")
        }
        return ""
    }
    function act(method, id) {
        Daemon.call(method, { id: id }, (ok, r, e) => {
            if (!ok)
                Announcer.announce(root, e.message, true)
        })
    }

    Subscription {
        topics: ["download.progress"]
        onReceived: (topic, d) => {
            const before = root.rows.find(r => r.id === d.id)
            root.seed([d])
            if (d.status === "done" && (!before || before.status !== "done"))
                root.finished(d.id)
            if (d.status === "waiting_network" && (!before || before.status !== "waiting_network"))
                Announcer.announce(root, root.statusText(d), false)
        }
    }

    Repeater {
        model: root.rows.filter(r => r.status !== "cancelled")
        ColumnLayout {
            required property var modelData
            readonly property var r: modelData
            Layout.fillWidth: true
            spacing: Theme.s1
            Progress {
                Layout.fillWidth: true
                label: root.names[r.id] || r.name || r.id
                bytes: r.bytes || 0
                total: r.total || 0
                rate: r.status === "downloading" ? (r.rate || 0) : 0
                etaS: r.status === "downloading" && r.etaS !== null && r.etaS !== undefined ? r.etaS : -1
                status: r.status
                indeterminate: r.status === "verifying" || r.status === "extracting" || (r.total || 0) <= 0
                activity: root.statusText(r)
            }
            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.s2
                Label {
                    Layout.fillWidth: true
                    visible: text.length > 0
                    text: root.statusText(r)
                    role: "small"
                    color: r.status === "failed" ? Theme.danger : r.status === "done" ? Theme.success : Theme.textSecondary
                }
                Item {
                    Layout.fillWidth: true
                    visible: root.statusText(r).length === 0
                }
                Button {
                    visible: r.status === "downloading" || r.status === "queued"
                    text: qsTr("Pause")
                    Accessible.name: qsTr("Pause %1").arg(root.names[r.id] || r.name || r.id)
                    onClicked: root.act("models.pause", r.id)
                }
                Button {
                    visible: r.status === "paused" || r.status === "waiting_network" || r.status === "failed"
                    text: r.status === "failed" ? qsTr("Try again") : qsTr("Resume now")
                    Accessible.name: qsTr("Resume %1").arg(root.names[r.id] || r.name || r.id)
                    onClicked: root.act(r.status === "failed" ? "models.download" : "models.resume", r.id)
                }
                Button {
                    visible: r.status !== "done" && r.status !== "failed" && r.status !== "verifying" && r.status !== "extracting"
                    variant: "quiet"
                    text: qsTr("Cancel")
                    Accessible.name: qsTr("Cancel %1").arg(root.names[r.id] || r.name || r.id)
                    onClicked: root.act("models.cancel", r.id)
                }
            }
        }
    }
}
