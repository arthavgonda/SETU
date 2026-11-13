import QtQuick
import Setu.Core
import Setu.Theme

Item {
    id: wave
    property bool live: visible
    property string mode: "listen"
    property real level: 0
    property real phase: 0
    property int bars: 9
    readonly property bool speaking: mode === "speak"
    readonly property color from: speaking ? Theme.notchSpeak : Theme.notchListen
    readonly property color to: speaking ? Theme.notchSpeakDeep : Theme.notchListenDeep
    readonly property real gain: speaking ? 3.4 : 5.5
    implicitWidth: Theme.px(64)
    implicitHeight: Theme.px(24)
    Accessible.role: Accessible.Indicator
    Accessible.name: speaking ? qsTr("Speaking") : qsTr("Microphone level")

    function mix(a, b, t) {
        return Qt.rgba(a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t, 1)
    }

    Subscription {
        topics: [wave.speaking ? "tts.level" : "audio.level"]
        active: wave.live
        onReceived: (topic, data) => {
            const raw = topic === "tts.level" ? Number(data.level) : Number(data.rms)
            wave.level = Math.min(1, Math.max(0, raw * wave.gain))
        }
    }
    Behavior on level {
        enabled: !Motion.reduced
        NumberAnimation { duration: 90; easing.type: Easing.OutQuad }
    }
    onLiveChanged: if (!live) level = 0
    onModeChanged: level = 0

    NumberAnimation on phase {
        running: wave.live && !Motion.reduced
        from: 0
        to: Math.PI * 2
        duration: wave.speaking ? 760 : 1100
        loops: Animation.Infinite
    }

    Row {
        anchors.centerIn: parent
        spacing: Math.max(2, Math.round(wave.width / (wave.bars * 3.2)))
        Repeater {
            model: wave.bars
            Rectangle {
                id: bar
                required property int index
                readonly property real t: wave.bars > 1 ? bar.index / (wave.bars - 1) : 0
                readonly property real hump: Math.sin(Math.PI * (bar.index + 0.5) / wave.bars)
                readonly property real swing: 0.5 + 0.5 * Math.sin(wave.phase * (1.3 + (bar.index % 4) * 0.41) + bar.index * 1.9)
                readonly property real rest: wave.live ? 0.18 + 0.3 * hump * swing : 0.16
                width: Math.max(2, Math.round(wave.width / (wave.bars * 1.9)))
                height: Math.max(width, wave.height * Math.min(1, rest + hump * wave.level * (0.55 + 0.6 * swing)))
                radius: width / 2
                anchors.verticalCenter: parent.verticalCenter
                color: wave.mix(wave.from, wave.to, bar.t)
                opacity: wave.live ? 1 : 0.5
                Behavior on color {
                    enabled: !Motion.reduced
                    ColorAnimation { duration: 260 }
                }
            }
        }
    }
}
