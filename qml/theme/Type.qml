pragma Singleton
import QtQuick
import Setu.Core

QtObject {
    readonly property bool reading: (Settings.values["ui.readingFont"] ?? "default") === "atkinson"
    readonly property string sans: reading ? "Atkinson Hyperlegible" : "IBM Plex Sans"
    readonly property string serif: "Source Serif 4"
    readonly property string mono: "IBM Plex Mono"
    readonly property real s: Theme.textScale

    readonly property var display: ({ family: serif, size: Math.round(28 * s), line: Math.round(34 * s), weight: Font.DemiBold })
    readonly property var title: ({ family: sans, size: Math.round(20 * s), line: Math.round(28 * s), weight: Font.DemiBold })
    readonly property var heading: ({ family: sans, size: Math.round(16 * s), line: Math.round(24 * s), weight: Font.DemiBold })
    readonly property var body: ({ family: sans, size: Math.round(14 * s), line: Math.round(20 * s), weight: Font.Normal })
    readonly property var label: ({ family: sans, size: Math.round(13 * s), line: Math.round(18 * s), weight: Font.Medium })
    readonly property var small: ({ family: sans, size: Math.max(12, Math.round(12 * s)), line: Math.round(16 * s), weight: Font.Normal })
    readonly property var code: ({ family: mono, size: Math.round(13 * s), line: Math.round(20 * s), weight: Font.Normal })
    readonly property var caption: ({ family: sans, size: Math.round(((Settings.values["ui.largeCaptions"] ?? false) ? 22 : 16) * s), line: Math.round(((Settings.values["ui.largeCaptions"] ?? false) ? 30 : 24) * s), weight: Font.Medium })

    function role(name) {
        switch (name) {
        case "display": return display
        case "title": return title
        case "heading": return heading
        case "label": return label
        case "small": return small
        case "mono": return code
        case "caption": return caption
        default: return body
        }
    }
}
