import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

ColumnLayout {
    id: root
    property bool running: false
    property var items: []
    property var result: null
    property string error
    signal completed(var result)
    spacing: Theme.s2

    function run() {
        items = []
        result = null
        error = ""
        running = true
        Daemon.call("benchmark.run", {}, (ok, r, e) => {
            if (!ok && e.code !== "busy") {
                root.running = false
                root.error = e.message
            }
        })
    }
    function cancel() {
        Daemon.call("benchmark.cancel", {})
        running = false
    }
    function forceActiveFocus() {
        runButton.forceActiveFocus()
    }

    Subscription {
        topics: ["benchmark.progress", "benchmark.result"]
        onReceived: (topic, d) => {
            if (topic === "benchmark.progress") {
                root.running = true
                let list = root.items.slice()
                const i = list.findIndex(x => x.item === d.item)
                if (i >= 0)
                    list[i] = d
                else
                    list.push(d)
                root.items = list
                if (d.status === "done")
                    Announcer.announce(root, d.label + ": " + d.value + (d.unit ? " " + d.unit : ""), false)
            } else {
                root.running = false
                root.result = d
                root.items = d.items.map(x => Object.assign({ status: "done" }, x))
                Announcer.announce(root, qsTr("Scan complete."), false)
                root.completed(d)
            }
        }
    }

    Repeater {
        model: root.items
        RowLayout {
            required property var modelData
            Layout.fillWidth: true
            spacing: Theme.s3
            Accessible.role: Accessible.StaticText
            Accessible.name: modelData.label + ": " + (modelData.status === "running" ? qsTr("measuring") : modelData.value + (modelData.unit ? " " + modelData.unit : ""))
            Icon {
                name: modelData.status === "done" ? "check" : modelData.status === "skipped" ? "minus" : "loader-circle"
                color: modelData.status === "done" ? Theme.success : Theme.textSecondary
                Accessible.ignored: true
            }
            Label {
                Layout.preferredWidth: Theme.px(200)
                text: modelData.label
                Accessible.ignored: true
            }
            Label {
                Layout.fillWidth: true
                text: modelData.status === "running" ? qsTr("Measuring") : modelData.status === "skipped" ? qsTr("Skipped") : String(modelData.value) + (modelData.unit ? " " + modelData.unit : "")
                role: modelData.status === "done" ? "mono" : "body"
                color: modelData.status === "done" ? Theme.textPrimary : Theme.textSecondary
                elide: Text.ElideRight
                Accessible.ignored: true
            }
        }
    }
    Label {
        Layout.fillWidth: true
        visible: root.result !== null
        text: qsTr("Scan complete.")
        font.weight: Font.DemiBold
    }
    Label {
        Layout.fillWidth: true
        visible: root.error.length > 0
        text: root.error
        color: Theme.danger
    }
    RowLayout {
        spacing: Theme.s2
        Button {
            id: runButton
            objectName: "setting:benchmark"
            visible: !root.running
            text: root.result || root.items.length > 0 ? qsTr("Run benchmark again") : qsTr("Run benchmark")
            iconName: "gauge"
            onClicked: root.run()
        }
        Button {
            visible: root.running
            text: qsTr("Cancel")
            onClicked: root.cancel()
        }
    }
}
