import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

WizardStep {
    id: step
    key: "microphone"
    title: qsTr("Say something")
    subtitle: qsTr("Let's make sure I can hear you.")
    summary: qsTr("Microphone check. The ring grows when the microphone hears the user. They can pick another microphone.")
    property var inputs: []
    property real peak: 0
    property real level: 0
    property real started: Date.now()
    property real now: Date.now()
    readonly property bool denied: Daemon.mic.permission === "denied"
    readonly property string outcome: {
        if (denied || !Daemon.mic.available)
            return "none"
        if (peak >= 0.02)
            return "good"
        if (peak >= 0.004)
            return "quiet"
        return now - started > 8000 ? "none" : "waiting"
    }
    canContinue: !denied && Daemon.mic.available
    screenState: ({ heard: outcome })
    voiceChoices: [{ id: "next", label: qsTr("continue"), phrases: ["continue", "you can hear me", "sounds good"] }]
    function voiceChoice(id) {
        if (id === "next")
            advance()
    }
    function load() {
        Daemon.call("audio.devices", {}, (ok, r) => {
            if (ok)
                step.inputs = r.inputs
        })
    }
    function reset() {
        peak = 0
        started = Date.now()
        now = started
    }
    Component.onCompleted: load()
    Connections {
        target: Daemon
        function onReconnected() { step.load() }
    }
    Subscription {
        topics: ["audio.level"]
        active: step.visible
        onReceived: (topic, d) => {
            const r = Number(d.rms)
            step.level = r
            if (r > step.peak)
                step.peak = r
        }
    }
    Timer {
        interval: 1000
        repeat: true
        running: step.visible
        onTriggered: step.now = Date.now()
    }
    onOutcomeChanged: if (outcome === "good" || outcome === "none") Announcer.announce(result, result.text, outcome === "none")

    Label {
        Layout.fillWidth: true
        horizontalAlignment: Text.AlignHCenter
        text: step.title
        role: "display"
        Accessible.role: Accessible.Heading
    }
    Item {
        Layout.alignment: Qt.AlignHCenter
        Layout.preferredWidth: Theme.px(200)
        Layout.preferredHeight: Theme.px(200)
        Rectangle {
            anchors.centerIn: parent
            width: Theme.px(120)
            height: width
            radius: width / 2
            color: step.outcome === "good" ? Theme.successBg : Theme.bgSunken
            scale: 1 + Math.min(0.6, step.level * 9)
            Behavior on scale {
                NumberAnimation { duration: 120; easing.type: Easing.OutBack; easing.overshoot: 2 }
            }
            Behavior on color {
                ColorAnimation { duration: Motion.standard }
            }
        }
        Rectangle {
            anchors.centerIn: parent
            width: Theme.px(84)
            height: width
            radius: width / 2
            color: step.outcome === "good" ? Theme.success : Theme.accent
            Behavior on color {
                ColorAnimation { duration: Motion.standard }
            }
            Icon {
                anchors.centerIn: parent
                name: step.outcome === "good" ? "check" : "mic"
                size: Theme.px(36)
                color: Theme.textOnAccent
            }
        }
    }
    StatusDot {
        id: result
        Layout.alignment: Qt.AlignHCenter
        showIcon: true
        level: step.outcome === "good" ? "success" : step.outcome === "quiet" ? "warning" : step.outcome === "none" ? "danger" : "info"
        text: step.outcome === "good" ? qsTr("I can hear you") : step.outcome === "quiet" ? qsTr("A bit quiet. Move closer.") : step.outcome === "none" ? (step.denied ? qsTr("Microphone access is off") : qsTr("No sound yet")) : qsTr("Listening…")
    }
    SettingSelect {
        Layout.fillWidth: true
        key: "audio.inputDevice"
        label: qsTr("Microphone")
        options: [{ value: "", text: qsTr("System default") }].concat(step.inputs.map(d => ({ value: d.id, text: d.name })))
        onActivated: step.reset()
    }
    SettingSlider {
        visible: step.outcome === "quiet"
        key: "audio.gain"
        label: qsTr("Input gain")
        from: 0.5
        to: 4
        stepSize: 0.1
        format: v => qsTr("%1x").arg(v.toFixed(1))
        onMoved: step.reset()
    }
    Button {
        Layout.alignment: Qt.AlignHCenter
        visible: step.outcome === "none"
        text: qsTr("Open privacy settings")
        iconName: "external-link"
        onClicked: Daemon.call("permissions.request", { item: "microphone" }, (ok, r, e) => {
            if (!ok)
                step.toast(e.message)
        })
    }
}
