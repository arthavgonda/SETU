import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

WizardStep {
    id: step
    key: "preferences"
    title: qsTr("Make it yours")
    subtitle: ""
    readonly property real rate: Number(Settings.values["tts.rate"] ?? 1)
    readonly property string speed: rate < 0.93 ? "slow" : rate > 1.07 ? "fast" : "normal"
    readonly property string confirmChoice: {
        const m = Settings.values["confirm.method"] ?? "any"
        return m === "voice" || m === "key" ? m : "any"
    }
    summary: qsTr("Preferences: how fast Setu talks, how to confirm risky actions, and whether Setu starts with the computer.")
    screenState: ({ speed: speed, confirm: confirmChoice, startAtLogin: Settings.values["startup.atLogin"] !== false })
    voiceChoices: [
        { id: "fast", label: qsTr("faster"), hint: "faster, quicker, speed up", phrases: ["faster", "talk faster", "speak faster"] },
        { id: "slow", label: qsTr("slower"), hint: "slower, slow down", phrases: ["slower", "talk slower", "speak slower"] },
        { id: "normal", label: qsTr("normal speed"), phrases: ["normal speed", "normal"] },
        { id: "voice", label: qsTr("confirm by voice"), hint: "confirm, voice, speaking, saying yes", phrases: ["confirm by voice"] },
        { id: "key", label: qsTr("confirm with keyboard"), hint: "confirm, keyboard, typing, key", phrases: ["confirm with keyboard"] },
        { id: "any", label: qsTr("confirm either way"), phrases: ["either way", "both"] },
        { id: "login_on", label: qsTr("start with my computer"), hint: "start automatically, startup, login, boot", phrases: ["start with my computer"] },
        { id: "login_off", label: qsTr("don't start with my computer"), phrases: ["don't start automatically"] },
        { id: "sample", label: qsTr("let me hear you"), hint: "hear, sample, listen, test", phrases: ["let me hear you", "play a sample"] }
    ]
    function setSpeed(id) {
        Settings.set("tts.rate", id === "fast" ? 1.2 : id === "slow" ? 0.85 : 1.0)
        sample.restart()
    }
    function voiceChoice(id) {
        if (id === "fast" || id === "slow" || id === "normal")
            setSpeed(id)
        else if (id === "voice" || id === "key" || id === "any")
            Settings.set("confirm.method", id)
        else if (id === "login_on" || id === "login_off")
            Settings.set("startup.atLogin", id === "login_on")
        else if (id === "sample")
            Daemon.call("voice.sample", {})
    }
    Timer {
        id: sample
        interval: 350
        onTriggered: Daemon.call("voice.sample", { text: qsTr("This is how fast I'll talk.") })
    }

    StepHeader {
        Layout.fillWidth: true
        title: step.title
    }
    Card {
        Layout.fillWidth: true
        Label {
            text: qsTr("How fast I talk")
            role: "label"
        }
        RowLayout {
            spacing: Theme.s2
            Repeater {
                model: [{ id: "slow", t: qsTr("Slower") }, { id: "normal", t: qsTr("Normal") }, { id: "fast", t: qsTr("Faster") }]
                Pill {
                    required property var modelData
                    text: modelData.t
                    selected: step.speed === modelData.id
                    onClicked: step.setSpeed(modelData.id)
                }
            }
        }
        Label {
            Layout.topMargin: Theme.s2
            text: qsTr("Confirm risky actions")
            role: "label"
        }
        RowLayout {
            spacing: Theme.s2
            Repeater {
                model: [{ id: "voice", t: qsTr("By voice"), i: "mic" }, { id: "key", t: qsTr("Keyboard"), i: "keyboard" }, { id: "any", t: qsTr("Either") }]
                Pill {
                    required property var modelData
                    text: modelData.t
                    iconName: modelData.i || ""
                    selected: step.confirmChoice === modelData.id
                    onClicked: Settings.set("confirm.method", modelData.id)
                }
            }
        }
        SettingSwitch {
            Layout.topMargin: Theme.s2
            key: "startup.atLogin"
            text: qsTr("Start with my computer")
        }
        SettingSwitch {
            key: "ui.readAloud"
            text: qsTr("Explain screens out loud")
        }
    }
}
