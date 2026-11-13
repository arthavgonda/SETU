import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

ColumnLayout {
    id: panel
    property string paragraph: ""
    property int count: 0
    property string phase: "idle"
    property string message: ""
    property bool hearing: false
    readonly property bool done: phase === "done"
    signal finished
    signal toast(string text)
    spacing: Theme.s4

    function start() {
        phase = "preparing"
        count = 0
        message = ""
        Daemon.call("onboarding.paragraph", {}, (ok, r, e) => {
            panel.paragraph = ok ? r.text : ""
            if (!ok) {
                panel.phase = "failed"
                panel.message = e.message
                return
            }
            Daemon.call("speaker.enroll", { text: panel.paragraph }, (ok2, r2, e2) => {
                if (!ok2) {
                    panel.phase = "failed"
                    panel.message = e2.message
                    return
                }
                panel.phase = "listening"
            })
        })
    }
    function stop() {
        Daemon.call("onboarding.converse", { screen: null })
    }

    Subscription {
        topics: ["speaker.enroll"]
        onReceived: (topic, d) => {
            if (d.phase === "hearing") {
                panel.hearing = true
                quiet.restart()
                return
            }
            panel.hearing = false
            if (d.phase === "sample") {
                panel.count = d.count
                Announcer.announce(panel, qsTr("%1 of 3 done.").arg(d.count), false)
            } else if (d.phase === "done") {
                panel.count = 3
                panel.phase = "done"
                Announcer.announce(panel, qsTr("Your voice is enrolled."), false)
                doneTimer.start()
            } else if (d.phase === "failed") {
                panel.phase = "failed"
                panel.message = d.message || ""
                Announcer.announce(panel, panel.message, true)
            }
        }
    }
    Timer {
        id: quiet
        interval: 1600
        onTriggered: panel.hearing = false
    }
    Timer {
        id: doneTimer
        interval: Motion.reduced ? 400 : 1400
        onTriggered: panel.finished()
    }

    Card {
        Layout.fillWidth: true
        pad: Theme.s5
        highlighted: panel.hearing
        LoadingState {
            Layout.fillWidth: true
            visible: panel.phase === "preparing"
            what: qsTr("Your paragraph")
        }
        Label {
            Layout.fillWidth: true
            visible: panel.paragraph.length > 0
            text: panel.paragraph
            role: "title"
            font.weight: Font.Normal
            opacity: panel.done ? 0.45 : 1
            Behavior on opacity {
                NumberAnimation { duration: Motion.standard }
            }
        }
    }

    RowLayout {
        Layout.alignment: Qt.AlignHCenter
        spacing: Theme.s4
        Accessible.role: Accessible.ProgressBar
        Accessible.name: qsTr("%1 of 3 readings done").arg(panel.count)
        Repeater {
            model: 3
            Item {
                id: dot
                required property int index
                readonly property bool filled: panel.count > index
                readonly property bool current: panel.count === index && panel.phase === "listening"
                implicitWidth: Theme.px(44)
                implicitHeight: Theme.px(44)
                Rectangle {
                    id: ring
                    anchors.centerIn: parent
                    width: parent.width
                    height: width
                    radius: width / 2
                    color: dot.filled ? Theme.accent : "transparent"
                    border.width: 2
                    border.color: dot.filled || dot.current ? Theme.accent : Theme.borderStrong
                    scale: dot.filled ? 1 : dot.current && panel.hearing ? 1.12 : 1
                    Behavior on scale {
                        NumberAnimation { duration: Motion.spring; easing.type: Easing.OutBack; easing.overshoot: Motion.overshoot * 2 }
                    }
                    Behavior on color {
                        ColorAnimation { duration: Motion.standard }
                    }
                    Label {
                        anchors.centerIn: parent
                        text: dot.filled ? "✓" : String(dot.index + 1)
                        role: "heading"
                        color: dot.filled ? Theme.textOnAccent : Theme.textPrimary
                    }
                }
                SequentialAnimation {
                    running: dot.filled && !Motion.reduced
                    NumberAnimation { target: ring; property: "scale"; from: 0.6; to: 1.18; duration: 180; easing.type: Easing.OutQuad }
                    NumberAnimation { target: ring; property: "scale"; to: 1; duration: Motion.spring; easing.type: Easing.OutBack; easing.overshoot: Motion.overshoot * 2 }
                }
            }
        }
    }

    Label {
        Layout.fillWidth: true
        horizontalAlignment: Text.AlignHCenter
        role: "small"
        color: panel.phase === "failed" ? Theme.danger : Theme.textSecondary
        text: panel.phase === "failed" ? panel.message : panel.done ? qsTr("Done") : panel.hearing ? qsTr("Listening…") : panel.phase === "listening" ? (panel.count === 0 ? qsTr("Read it aloud") : panel.count === 1 ? qsTr("Once more") : qsTr("One last time")) : ""
    }

    RowLayout {
        Layout.alignment: Qt.AlignHCenter
        spacing: Theme.s2
        Button {
            visible: panel.phase === "failed"
            variant: "primary"
            large: false
            iconName: "rotate-ccw"
            text: qsTr("Try again")
            onClicked: panel.start()
        }
        Button {
            visible: panel.phase === "listening" && panel.count === 0
            variant: "quiet"
            iconName: "refresh-cw"
            text: qsTr("New paragraph")
            onClicked: panel.start()
        }
    }
}
