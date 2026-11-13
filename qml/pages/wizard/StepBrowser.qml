import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

WizardStep {
    id: step
    key: "browser"
    title: items.length > 1 ? qsTr("Which browser is yours?") : qsTr("Your browser")
    subtitle: items.length > 1 ? qsTr("Say its name or pick one.") : ""
    property var items: []
    property bool loaded: false
    property string chosen
    property string phase: ""
    property string message: ""
    readonly property var chosenItem: items.find(b => b.id === chosen) || null
    summary: qsTr("Choosing the browser Setu uses. Setu works in it directly, so nothing needs to be installed.")
    spoken: items.length > 1 ? qsTr("Which browser should I use? %1? Say its name.").arg(items.map(b => b.name).join(", ")) : qsTr("I'll use your browser for tabs, pages and videos.")
    screenState: ({ browsers: items.map(b => b.name), chosen: chosenItem ? chosenItem.name : "", setup: phase })
    voiceChoices: items.map(b => ({ id: b.id, label: b.name, phrases: [b.name, b.key, b.name + " browser"] }))
    canContinue: loaded

    function voiceChoice(id) {
        choose(id)
    }

    function choose(id) {
        if (id === chosen && phase !== "failed")
            return
        chosen = id
        phase = "working"
        message = ""
        Daemon.call("sound.play", { name: "tap" })
        Daemon.call("browsers.choose", { id: id }, (ok, r, e) => {
            if (!ok) {
                step.phase = "failed"
                step.message = e.message
                return
            }
            step.phase = r.state
        })
    }

    function load() {
        Daemon.call("browsers.list", {}, (ok, r) => {
            step.loaded = true
            if (!ok)
                return
            step.items = r.items
            const picked = r.items.find(b => b.chosen)
            if (picked) {
                step.chosen = picked.id
                step.phase = picked.connected ? "connected" : ""
            } else if (r.items.length === 1) {
                step.choose(r.items[0].id)
            }
        })
    }

    Component.onCompleted: load()

    Subscription {
        topics: ["browsers.state"]
        onReceived: (topic, d) => {
            if (d.items)
                step.items = d.items
            if (d.setup) {
                step.phase = d.setup
                step.message = d.message || ""
            }
        }
    }

    StepHeader {
        Layout.fillWidth: true
        title: step.title
        subtitle: step.subtitle
    }

    Item {
        Layout.fillWidth: true
        Layout.preferredHeight: Theme.px(150)
        visible: step.items.length > 0
        Row {
            anchors.centerIn: parent
            spacing: Theme.s5
            Repeater {
                model: step.items
                Item {
                    id: tile
                    required property var modelData
                    required property int index
                    readonly property bool picked: modelData.id === step.chosen
                    width: Theme.px(104)
                    height: Theme.px(140)
                    activeFocusOnTab: true
                    Accessible.role: Accessible.Button
                    Accessible.name: modelData.name
                    Accessible.onPressAction: step.choose(modelData.id)
                    Keys.onReturnPressed: step.choose(modelData.id)
                    Keys.onSpacePressed: step.choose(modelData.id)
                    property real bob: 0
                    opacity: 0
                    scale: 0.4
                    Component.onCompleted: arrive.start()
                    SequentialAnimation {
                        id: arrive
                        PauseAnimation { duration: Motion.reduced ? 0 : 120 + tile.index * 110 }
                        ParallelAnimation {
                            NumberAnimation { target: tile; property: "opacity"; to: 1; duration: Motion.reduced ? 0 : 240 }
                            NumberAnimation { target: tile; property: "scale"; to: 1; duration: Motion.reduced ? 0 : 560; easing.type: Easing.OutBack; easing.overshoot: 2.2 }
                        }
                    }
                    SequentialAnimation on bob {
                        running: !Motion.reduced && (step.chosen.length === 0 || tile.picked && step.phase === "working")
                        loops: Animation.Infinite
                        PauseAnimation { duration: tile.index * 180 }
                        NumberAnimation { from: 0; to: -Theme.px(8); duration: 760; easing.type: Easing.InOutSine }
                        NumberAnimation { from: -Theme.px(8); to: 0; duration: 760; easing.type: Easing.InOutSine }
                    }
                    Rectangle {
                        id: card
                        anchors.horizontalCenter: parent.horizontalCenter
                        y: tile.bob
                        width: Theme.px(88)
                        height: width
                        radius: Theme.px(24)
                        color: tile.picked ? Theme.bgSelected : (hover.hovered ? Theme.bgHover : Theme.bgRaised)
                        border.width: tile.picked || tile.activeFocus ? Theme.focusWidth : Theme.border
                        border.color: tile.picked || tile.activeFocus ? Theme.accent : Theme.borderSubtle
                        scale: hover.hovered ? 1.06 : (step.chosen.length > 0 && !tile.picked ? 0.92 : 1)
                        opacity: step.chosen.length > 0 && !tile.picked ? 0.55 : 1
                        Behavior on scale {
                            NumberAnimation { duration: Motion.standard; easing.type: Easing.OutBack }
                        }
                        Behavior on opacity {
                            NumberAnimation { duration: Motion.standard }
                        }
                        Image {
                            id: tileImage
                            anchors.centerIn: parent
                            width: Theme.px(56)
                            height: width
                            sourceSize: Qt.size(width * 2, height * 2)
                            source: tile.modelData.icon && tile.modelData.icon.length > 0 ? "image://appicon/" + encodeURIComponent(tile.modelData.icon) : ""
                            fillMode: Image.PreserveAspectFit
                            smooth: true
                            Accessible.ignored: true
                        }
                        Icon {
                            anchors.centerIn: parent
                            visible: tileImage.status !== Image.Ready
                            name: "globe"
                            size: Theme.px(40)
                            color: Theme.textSecondary
                        }
                        Rectangle {
                            visible: tile.picked && step.phase === "connected"
                            anchors.right: parent.right
                            anchors.bottom: parent.bottom
                            anchors.margins: -Theme.px(4)
                            width: Theme.px(26)
                            height: width
                            radius: width / 2
                            color: Theme.success
                            Icon {
                                anchors.centerIn: parent
                                name: "check"
                                size: Theme.px(16)
                                color: Theme.textOnAccent
                            }
                        }
                        HoverHandler {
                            id: hover
                            cursorShape: Qt.PointingHandCursor
                        }
                        TapHandler {
                            onTapped: step.choose(tile.modelData.id)
                        }
                    }
                    Label {
                        anchors.horizontalCenter: parent.horizontalCenter
                        anchors.bottom: parent.bottom
                        text: tile.modelData.name
                        font.weight: tile.picked ? Font.DemiBold : Font.Normal
                        color: tile.picked ? Theme.textPrimary : Theme.textSecondary
                    }
                }
            }
        }
    }

    Label {
        Layout.fillWidth: true
        horizontalAlignment: Text.AlignHCenter
        visible: step.loaded && step.items.length === 0
        text: qsTr("I couldn't find a browser. You can still use me for everything else, and I can install one when you ask.")
        color: Theme.textSecondary
    }

    RowLayout {
        Layout.alignment: Qt.AlignHCenter
        visible: step.phase.length > 0
        spacing: Theme.s2
        Icon {
            visible: step.phase === "working"
            name: "loader-circle"
            color: Theme.textSecondary
            RotationAnimation on rotation {
                running: step.phase === "working" && !Motion.reduced
                from: 0
                to: 360
                duration: 900
                loops: Animation.Infinite
            }
        }
        Icon {
            visible: step.phase === "connected"
            name: "circle-check"
            color: Theme.success
        }
        Label {
            Layout.maximumWidth: Theme.px(520)
            horizontalAlignment: Text.AlignHCenter
            color: step.phase === "failed" ? Theme.danger : Theme.textSecondary
            text: {
                const name = step.chosenItem ? step.chosenItem.name : qsTr("your browser")
                switch (step.phase) {
                case "working": return qsTr("Setting up %1.").arg(name)
                case "connected": return qsTr("%1 is ready.").arg(name)
                case "guided": return step.message.length > 0 ? step.message : qsTr("%1 needs one last click from you.").arg(name)
                case "failed": return step.message.length > 0 ? step.message : qsTr("I couldn't finish setting up %1. You can continue and ask me later.").arg(name)
                }
                return ""
            }
        }
    }
}
