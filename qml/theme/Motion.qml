pragma Singleton
import QtQuick
import Setu.Core

QtObject {
    readonly property bool reduced: Platform.reduceMotion || (Settings.values["ui.reduceMotion"] ?? false)
    readonly property int micro: 100
    readonly property int standard: reduced ? 0 : 180
    readonly property int overlay: reduced ? 0 : 280
    readonly property int fade: reduced ? 100 : 180
    readonly property int spring: reduced ? 0 : 420
    readonly property int morph: reduced ? 0 : 520
    readonly property int stagger: reduced ? 0 : 45
    readonly property real overshoot: 1.6
    readonly property var curve: [0.2, 0, 0, 1, 1, 1]
    readonly property real timeoutScale: {
        const v = Settings.values["ui.timeoutScale"] ?? "1"
        return v === "none" ? 0 : Number(v)
    }
}
