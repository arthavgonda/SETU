import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

Page {
    id: page
    property var inputs: []
    property var outputs: []
    property var voices: []
    property var speakers: ({})
    property bool sampling: false

    function load() {
        Daemon.call("audio.devices", {}, (ok, r) => {
            if (!ok)
                return
            page.inputs = r.inputs
            page.outputs = r.outputs
        })
        Daemon.call("models.list", {}, (ok, r) => {
            if (!ok)
                return
            const names = {}
            for (const c of r.catalog)
                names[c.id] = c
            let sp = {}
            for (const c of r.catalog)
                for (const v of (c.speakers || []))
                    sp[v.id] = v.name + "  ·  " + v.accent
            page.speakers = sp
            page.voices = r.installed.filter(m => m.slot === "tts" && m.verified).map(m => ({ value: m.id, text: m.name, size: names[m.id] ? names[m.id].sizeBytes : 0 }))
        })
    }
    function deviceOptions(list) {
        return [{ value: "", text: qsTr("System default") }].concat(list.map(d => ({ value: d.id, text: d.isDefault ? qsTr("%1 (default)").arg(d.name) : d.name })))
    }
    readonly property string currentVoice: {
        const s = Daemon.model("tts")
        return s && s.model ? s.model : (Settings.values["tts.voice"] || "")
    }

    Component.onCompleted: if (Daemon.connected) load()
    Connections {
        target: Daemon
        function onReconnected() { page.load() }
    }

    ModelLibrary {
        id: library
        immediate: true
        onToast: text => page.toast(text)
        onPicked: page.load()
    }
    Dialog {
        id: enrollDialog
        title: qsTr("Teach me your voice")
        preferredWidth: Theme.px(620)
        onOpened: enrollPanel.start()
        onClosed: enrollPanel.stop()
        EnrollPanel {
            id: enrollPanel
            Layout.fillWidth: true
            onToast: text => page.toast(text)
            onFinished: enrollDialog.close()
        }
        buttons: [
            Item { Layout.fillWidth: true },
            Button {
                text: enrollPanel.done ? qsTr("Done") : qsTr("Cancel")
                onClicked: enrollDialog.close()
            }
        ]
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("Microphone")
        SettingSelect {
            objectName: "setting:audio.inputDevice"
            key: "audio.inputDevice"
            label: qsTr("Input device")
            options: page.deviceOptions(page.inputs)
        }
        Trace {
            Layout.fillWidth: true
            live: page.visible && Daemon.connected && !Daemon.mic.muted
        }
        Label {
            Layout.fillWidth: true
            role: "small"
            color: Daemon.mic.available ? Theme.textSecondary : Theme.danger
            text: Daemon.mic.permission === "denied" ? qsTr("%1 can't hear you. Microphone access is off.").arg(Brand.productName) : !Daemon.mic.available ? qsTr("No microphone found. Plug one in or choose a device.") : Daemon.mic.muted ? qsTr("The microphone is off.") : qsTr("The line moves when the microphone hears sound.")
        }
        SettingSlider {
            objectName: "setting:audio.gain"
            key: "audio.gain"
            label: qsTr("Input gain")
            from: 0.5
            to: 4
            stepSize: 0.1
            format: v => qsTr("%1x").arg(v.toFixed(1))
        }
        SettingSwitch {
            objectName: "setting:audio.noiseReduction"
            key: "audio.noiseReduction"
            text: qsTr("Noise reduction")
        }
        SettingSwitch {
            objectName: "setting:audio.quietVoice"
            key: "audio.quietVoice"
            text: qsTr("Quiet-voice mode")
        }
        TextField {
            id: stopWord
            objectName: "setting:voice.stopWord"
            Layout.maximumWidth: Theme.px(280)
            Layout.fillWidth: true
            label: qsTr("Stop word")
            maximumLength: 24
            text: Settings.values["voice.stopWord"] || "stop"
            error: /^\s*[^\s\d.,;:!?"()]+\s*$/.test(text) ? "" : qsTr("Use a single word.")
            onAccepted: if (error.length === 0) Settings.set("voice.stopWord", text.trim().toLowerCase())
            field.onEditingFinished: if (error.length === 0 && text.trim().toLowerCase() !== Settings.values["voice.stopWord"]) Settings.set("voice.stopWord", text.trim().toLowerCase())
        }
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("Speech")
        SettingSelect {
            objectName: "setting:audio.outputDevice"
            key: "audio.outputDevice"
            label: qsTr("Output device")
            options: page.deviceOptions(page.outputs)
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s2
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2
                Label {
                    text: qsTr("Voice")
                    role: "label"
                }
                Label {
                    Layout.fillWidth: true
                    text: {
                        const s = Daemon.model("tts") || {}
                        return (s.name || "") + (s.voice ? "  ·  " + (page.speakers[s.voice] || s.voice) : "")
                    }
                    color: Theme.textSecondary
                }
            }
            Button {
                objectName: "setting:tts.voice"
                text: qsTr("Change")
                onClicked: library.show("tts")
            }
            Button {
                text: qsTr("Play sample")
                iconName: "volume-2"
                busy: page.sampling
                enabled: !page.sampling
                onClicked: {
                    page.sampling = true
                    Daemon.call("voice.sample", {}, (ok, r, e) => {
                        if (!ok) {
                            page.sampling = false
                            page.toast(e.message)
                            return
                        }
                        sampleDone.interval = Math.max(500, r.durationMs || 3000)
                        sampleDone.restart()
                    })
                }
                Timer {
                    id: sampleDone
                    onTriggered: page.sampling = false
                }
            }
        }
        SettingSlider {
            objectName: "setting:tts.rate"
            key: "tts.rate"
            label: qsTr("Speech rate")
            from: 0.5
            to: 2
            stepSize: 0.05
            lowText: qsTr("Slower")
            highText: qsTr("Faster")
            format: v => qsTr("%1x").arg(v.toFixed(2))
        }
        SettingSlider {
            objectName: "setting:tts.volume"
            key: "tts.volume"
            label: qsTr("Volume")
            from: 0
            to: 1
            stepSize: 0.05
            format: v => Math.round(v * 100) + "%"
        }
        SettingSwitch {
            objectName: "setting:tts.speakReplies"
            key: "tts.speakReplies"
            text: qsTr("Speak replies")
        }
        SettingSwitch {
            objectName: "setting:audio.listeningSound"
            key: "audio.listeningSound"
            text: qsTr("Play listening sound")
        }
        SettingSwitch {
            objectName: "setting:tts.readConfirmations"
            key: "tts.readConfirmations"
            text: qsTr("Read confirmations aloud")
        }
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("Listening window")
        SettingSlider {
            objectName: "setting:session.activeMinutes"
            key: "session.activeMinutes"
            label: qsTr("Active window length")
            from: 1
            to: 60
            stepSize: 1
            format: v => qsTr("%n minutes", "", Math.round(v))
        }
        SettingSlider {
            objectName: "setting:session.graceSeconds"
            key: "session.graceSeconds"
            label: qsTr("Flow grace period")
            from: 1
            to: 30
            stepSize: 1
            format: v => qsTr("%n seconds", "", Math.round(v))
        }
        SettingSlider {
            objectName: "setting:session.endSilence"
            key: "session.endSilence"
            label: qsTr("End-of-speech silence")
            from: 0.5
            to: 4
            stepSize: 0.1
            lowText: qsTr("Answers sooner")
            highText: qsTr("Waits longer for slow speech")
            format: v => qsTr("%1 s").arg(v.toFixed(1))
        }
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("Only my voice")
        SettingSwitch {
            objectName: "setting:speaker.verification"
            key: "speaker.verification"
            enabled: Daemon.speakerEnrolled || checked
            text: qsTr("Only respond to my voice")
            description: Daemon.speakerEnrolled ? "" : qsTr("Teach me your voice first.")
        }
        Button {
            text: Daemon.speakerEnrolled ? qsTr("Teach me again") : qsTr("Teach me your voice")
            iconName: "mic"
            onClicked: enrollDialog.open()
        }
    }
}
