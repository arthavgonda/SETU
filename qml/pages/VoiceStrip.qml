import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

Item {
    id: strip
    property bool listening: false
    property bool thinking: false
    property string setuText
    property string heardText
    property string shown: heardText.length > 0 ? heardText : setuText
    property real level: 0
    readonly property bool fromUser: heardText.length > 0
    implicitHeight: Math.max(Theme.px(44), bubble.implicitHeight)
    implicitWidth: Theme.px(320)
    Accessible.role: Accessible.StaticText
    Accessible.name: listening ? qsTr("Listening. %1").arg(shown) : shown

    onShownChanged: if (shown.length > 0) pop.restart()
    Subscription {
        topics: ["audio.level"]
        active: strip.listening && strip.visible
        onReceived: (topic, d) => strip.level = Number(d.rms) || 0
    }
    onHeardTextChanged: if (heardText.length > 0) clearHeard.restart()
    Timer {
        id: clearHeard
        interval: 6000
        onTriggered: strip.heardText = ""
    }

    RowLayout {
        anchors.fill: parent
        spacing: Theme.s3
        Item {
            Layout.preferredWidth: Theme.px(36)
            Layout.preferredHeight: Theme.px(36)
            visible: strip.listening
            Rectangle {
                id: halo
                anchors.centerIn: parent
                width: parent.width
                height: width
                radius: width / 2
                color: Theme.accent
                opacity: 0.18
                scale: 1 + Math.min(0.5, strip.level * 6)
                Behavior on scale {
                    NumberAnimation { duration: 90 }
                }
            }
            Rectangle {
                anchors.centerIn: parent
                width: parent.width * 0.7
                height: width
                radius: width / 2
                color: Theme.accent
                Icon {
                    anchors.centerIn: parent
                    name: "mic"
                    size: Theme.px(16)
                    color: Theme.textOnAccent
                }
            }
        }
        Rectangle {
            id: bubble
            Layout.fillWidth: true
            Layout.maximumWidth: Theme.px(560)
            implicitHeight: Math.max(Theme.px(36), line.implicitHeight + Theme.s2 * 2)
            radius: Theme.radiusOverlay
            visible: strip.shown.length > 0 || strip.thinking
            color: strip.fromUser ? Theme.bgSunken : "transparent"
            border.width: strip.fromUser ? 0 : Theme.border
            border.color: Theme.borderSubtle
            transformOrigin: Item.Left
            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: Theme.s3
                anchors.rightMargin: Theme.s3
                spacing: Theme.s2
                Label {
                    id: line
                    Layout.fillWidth: true
                    text: strip.shown
                    role: "small"
                    color: strip.fromUser ? Theme.textPrimary : Theme.textSecondary
                    maximumLineCount: 2
                    elide: Text.ElideRight
                    font.italic: strip.fromUser
                }
                Row {
                    visible: strip.thinking
                    spacing: 4
                    Repeater {
                        model: 3
                        Rectangle {
                            required property int index
                            width: 6
                            height: 6
                            radius: 3
                            color: Theme.textSecondary
                            SequentialAnimation on opacity {
                                running: strip.thinking && !Motion.reduced
                                loops: Animation.Infinite
                                PauseAnimation { duration: index * 120 }
                                NumberAnimation { from: 0.25; to: 1; duration: 300 }
                                NumberAnimation { from: 1; to: 0.25; duration: 300 }
                                PauseAnimation { duration: (2 - index) * 120 }
                            }
                        }
                    }
                }
            }
            SequentialAnimation {
                id: pop
                NumberAnimation { target: bubble; property: "scale"; from: Motion.reduced ? 1 : 0.92; to: 1; duration: Motion.spring; easing.type: Easing.OutBack; easing.overshoot: Motion.overshoot * 2 }
            }
        }
    }
}
