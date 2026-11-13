import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

WizardStep {
    id: step
    key: "welcome"
    title: qsTr("Namaste, I'm %1").arg(Brand.assistantName)
    subtitle: qsTr("Your voice assistant. Private, on this computer.")
    summary: qsTr("Welcome screen. Choose a language and text size, then continue.")
    primaryText: qsTr("Get started")
    showBack: false
    readonly property string current: Settings.values["ui.language"] || Translator.language
    readonly property real textScale: Number(Settings.values["ui.textScale"] ?? 1)
    voiceChoices: [{ id: "next", label: qsTr("get started"), phrases: ["get started", "let's start", "start"] }]
    function voiceChoice(id) {
        if (id === "next")
            advance()
    }

    SetuFace {
        Layout.alignment: Qt.AlignHCenter
        Layout.preferredWidth: Theme.px(150)
        Layout.preferredHeight: Theme.px(72)
        mood: "happy"
    }
    ColumnLayout {
        Layout.fillWidth: true
        spacing: Theme.s2
        Label {
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
            text: step.title
            role: "display"
            Accessible.role: Accessible.Heading
        }
        Label {
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
            text: step.subtitle
            color: Theme.textSecondary
        }
    }
    Card {
        Layout.fillWidth: true
        Layout.topMargin: Theme.s3
        Select {
            Layout.fillWidth: true
            label: qsTr("Language")
            options: Translator.languages.map(l => ({ value: l.code, text: l.native === l.english ? l.native : l.native + "  ·  " + l.english }))
            value: step.current
            onActivated: v => {
                Settings.set("ui.language", v)
                Translator.apply(v)
            }
        }
        Segmented {
            Layout.fillWidth: true
            label: qsTr("Text size")
            value: step.textScale >= 1.9 ? 2 : step.textScale >= 1.4 ? 1.5 : step.textScale >= 1.2 ? 1.25 : 1
            options: [{ value: 1, text: "100%" }, { value: 1.25, text: "125%" }, { value: 1.5, text: "150%" }, { value: 2, text: "200%" }]
            onActivated: v => Settings.set("ui.textScale", v)
        }
        SettingSwitch {
            key: "ui.readAloud"
            text: qsTr("Explain screens out loud")
        }
    }
}
