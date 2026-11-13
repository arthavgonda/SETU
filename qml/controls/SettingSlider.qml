import QtQuick
import QtQuick.Layouts
import Setu.Core

Slider {
    id: s
    property string key
    Layout.fillWidth: true
    value: Number(Settings.values[key] ?? from)
    onMoved: v => commit.restart()
    Timer {
        id: commit
        interval: 250
        onTriggered: Settings.set(s.key, s.slider.value)
    }
}
