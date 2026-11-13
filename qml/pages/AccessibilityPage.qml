import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

Page {
    id: page

    readonly property string screenReaderHelp: {
        if (Platform.os === "macos")
            return qsTr("Turn VoiceOver on with Command + F5. Move through %1 with Tab, and through text with VoiceOver + arrow keys. Each page title and setting is read out, and changes in the listening overlay are announced once.").arg(Brand.productName)
        if (Platform.os === "windows")
            return qsTr("Turn Narrator on with Windows + Control + Enter, or start NVDA. Move through %1 with Tab and the arrow keys. Each page title and setting is read out, and changes in the listening overlay are announced once.").arg(Brand.productName)
        return qsTr("Turn Orca on with Super + Alt + S, or from Settings > Accessibility. Move through %1 with Tab and the arrow keys, and between areas with F6. Each page title and setting is read out, and changes in the listening overlay are announced once.").arg(Brand.productName)
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("Text and colour")
        SettingSlider {
            objectName: "setting:ui.textScale"
            key: "ui.textScale"
            label: qsTr("Text size")
            from: 1
            to: 2
            stepSize: 0.1
            format: v => Math.round(v * 100) + "%"
        }
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: preview.implicitHeight + Theme.s4 * 2
            radius: Theme.radiusPanel
            color: Theme.bgSunken
            border.width: Theme.border
            border.color: Theme.borderSubtle
            Accessible.role: Accessible.StaticText
            Accessible.name: qsTr("Preview: %1").arg(preview.text)
            Label {
                id: preview
                anchors.fill: parent
                anchors.margins: Theme.s4
                text: qsTr("Hey %1, open the browser. This is how text looks at this size.").arg(Brand.assistantName)
                Accessible.ignored: true
            }
        }
        Segmented {
            objectName: "setting:ui.theme"
            Layout.fillWidth: true
            label: qsTr("Theme")
            value: Settings.values["ui.theme"] ?? "system"
            options: [
                { value: "system", text: qsTr("Follow system") },
                { value: "light", text: qsTr("Light") },
                { value: "dark", text: qsTr("Dark") },
                { value: "contrast", text: qsTr("High contrast") }
            ]
            onActivated: v => Settings.set("ui.theme", v)
        }
        Segmented {
            objectName: "setting:ui.readingFont"
            Layout.fillWidth: true
            label: qsTr("Reading font")
            value: Settings.values["ui.readingFont"] ?? "default"
            options: [
                { value: "default", text: qsTr("Default") },
                { value: "atkinson", text: qsTr("Atkinson Hyperlegible") }
            ]
            onActivated: v => Settings.set("ui.readingFont", v)
        }
        SettingSwitch {
            objectName: "setting:ui.reduceMotion"
            key: "ui.reduceMotion"
            text: qsTr("Reduce motion")
            description: Platform.reduceMotion ? qsTr("Already on because your system asks for reduced motion.") : qsTr("Turns off animation in the overlay and windows.")
        }
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("Captions and announcements")
        Label {
            Layout.fillWidth: true
            text: qsTr("Captions are always on in the listening overlay.")
            color: Theme.textSecondary
        }
        SettingSwitch {
            objectName: "setting:ui.largeCaptions"
            key: "ui.largeCaptions"
            text: qsTr("Larger captions")
        }
        Segmented {
            objectName: "setting:ui.announceDetail"
            Layout.fillWidth: true
            label: qsTr("Screen reader announcements")
            value: Settings.values["ui.announceDetail"] ?? "normal"
            options: [
                { value: "brief", text: qsTr("Brief") },
                { value: "normal", text: qsTr("Normal") },
                { value: "verbose", text: qsTr("Verbose") }
            ]
            onActivated: v => Settings.set("ui.announceDetail", v)
        }
        SettingSlider {
            objectName: "setting:tts.rate.a11y"
            key: "tts.rate"
            label: qsTr("Speech rate")
            from: 0.5
            to: 2
            stepSize: 0.05
            format: v => qsTr("%1x").arg(v.toFixed(2))
        }
        Button {
            text: qsTr("Test with a screen reader")
            iconName: "speech"
            onClicked: srHelp.visible = !srHelp.visible
        }
        Label {
            id: srHelp
            Layout.fillWidth: true
            visible: false
            text: page.screenReaderHelp
            onVisibleChanged: if (visible) Announcer.announce(srHelp, text, false)
        }
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("Confirming and timing")
        Segmented {
            objectName: "setting:confirm.method"
            Layout.fillWidth: true
            label: qsTr("Confirmation method")
            value: Settings.values["confirm.method"] ?? "any"
            options: [
                { value: "voice", text: qsTr("Voice") },
                { value: "key", text: qsTr("Key") },
                { value: "switch", text: qsTr("Switch") },
                { value: "any", text: qsTr("Any") }
            ]
            onActivated: v => Settings.set("confirm.method", v)
        }
        Segmented {
            objectName: "setting:ui.timeoutScale"
            Layout.fillWidth: true
            label: qsTr("Timeout lengthening")
            value: Settings.values["ui.timeoutScale"] ?? "1"
            options: [
                { value: "1", text: qsTr("1x") },
                { value: "2", text: qsTr("2x") },
                { value: "3", text: qsTr("3x") },
                { value: "none", text: qsTr("No time limit") }
            ]
            onActivated: v => Settings.set("ui.timeoutScale", v)
        }
        Label {
            Layout.fillWidth: true
            role: "small"
            color: Theme.textSecondary
            text: qsTr("Applies to every timed question and confirmation. A confirmation is never accepted by itself when time runs out.")
        }
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("Notch")
        SettingSelect {
            objectName: "setting:ui.notch"
            key: "ui.notch"
            label: qsTr("Show the notch")
            options: [
                { value: "always", text: qsTr("Always") },
                { value: "active", text: qsTr("While Setu is listening or busy") },
                { value: "off", text: qsTr("Only for questions") }
            ]
        }
        SettingSwitch {
            objectName: "setting:media.lyrics"
            key: "media.lyrics"
            text: qsTr("Show song lyrics")
        }
        Label {
            Layout.fillWidth: true
            text: qsTr("While music plays, the song's title and artist are looked up on lrclib.net to show the words in time.")
            role: "small"
            color: Theme.textSecondary
        }
        SettingSwitch {
            objectName: "setting:ui.switchScanning"
            key: "ui.switchScanning"
            text: qsTr("Switch scanning")
        }
        Banner {
            Layout.fillWidth: true
            visible: Platform.overlayMode === "notification"
            level: "info"
            announce: false
            text: qsTr("Your desktop can't show a floating panel that stays on top without taking focus. The overlay is shown as notifications instead.")
        }
    }
}
