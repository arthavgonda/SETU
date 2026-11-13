import QtQuick
import Setu.Theme

Item {
    id: eq
    property bool playing: true
    property color ink: Theme.notchMusic
    property int bars: 4
    readonly property bool calm: Motion.reduced
    implicitHeight: Theme.px(16)
    implicitWidth: row.implicitWidth
    Accessible.role: Accessible.Indicator
    Accessible.name: playing ? qsTr("Playing") : qsTr("Paused")

    Row {
        id: row
        anchors.centerIn: parent
        height: eq.height
        spacing: Math.max(2, Math.round(eq.height / 7))
        Repeater {
            model: eq.bars
            Rectangle {
                id: bar
                required property int index
                readonly property var peaks: [0.95, 0.6, 0.85, 0.7, 0.9]
                readonly property var speeds: [430, 330, 510, 380, 460]
                width: Math.max(2, Math.round(eq.height / 5.5))
                height: Math.max(width, eq.height * 0.3)
                radius: width / 2
                anchors.verticalCenter: parent.verticalCenter
                color: eq.ink
                SequentialAnimation on height {
                    running: eq.playing && !eq.calm
                    loops: Animation.Infinite
                    NumberAnimation { to: eq.height * bar.peaks[bar.index % 5]; duration: bar.speeds[bar.index % 5]; easing.type: Easing.InOutSine }
                    NumberAnimation { to: eq.height * 0.25; duration: bar.speeds[(bar.index + 2) % 5]; easing.type: Easing.InOutSine }
                    NumberAnimation { to: eq.height * bar.peaks[(bar.index + 1) % 5] * 0.8; duration: bar.speeds[(bar.index + 1) % 5]; easing.type: Easing.InOutSine }
                    NumberAnimation { to: eq.height * 0.35; duration: bar.speeds[(bar.index + 3) % 5]; easing.type: Easing.InOutSine }
                }
                Behavior on height {
                    enabled: !eq.playing && !eq.calm
                    NumberAnimation { duration: 240; easing.type: Easing.OutQuad }
                }
                Connections {
                    target: eq
                    function onPlayingChanged() {
                        if (!eq.playing)
                            bar.height = Math.max(bar.width, eq.height * 0.3)
                    }
                }
            }
        }
    }
}
