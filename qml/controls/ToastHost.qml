import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme

Item {
    id: host
    LayoutMirroring.enabled: Platform.rightToLeft
    LayoutMirroring.childrenInherit: true
    property var queue: []
    property var log: []
    property string current: ""
    property bool paused: hover.hovered || closeButton.activeFocus
    implicitHeight: box.implicitHeight
    implicitWidth: box.implicitWidth
    visible: current.length > 0

    function show(text) {
        log = log.concat([{ time: Date.now(), text: text }])
        if (current.length === 0) {
            current = text
            Announcer.announce(host, text)
            timer.restart()
        } else {
            queue = queue.concat([text])
        }
    }

    function next() {
        if (queue.length > 0) {
            current = queue[0]
            queue = queue.slice(1)
            Announcer.announce(host, current)
            timer.restart()
        } else {
            current = ""
        }
    }

    Timer {
        id: timer
        interval: 4000 * Math.max(1, Motion.timeoutScale || 1)
        running: host.current.length > 0 && !host.paused && Motion.timeoutScale !== 0
        onTriggered: host.next()
    }
    HoverHandler {
        id: hover
    }
    Rectangle {
        id: box
        implicitWidth: row.implicitWidth + Theme.s4 * 2
        implicitHeight: row.implicitHeight + Theme.s2 * 2
        radius: Theme.radiusPanel
        color: Theme.textPrimary
        border.width: Theme.contrast ? Theme.border : 0
        border.color: Theme.bgRaised
        Accessible.role: Accessible.StaticText
        Accessible.name: host.current
        RowLayout {
            id: row
            anchors.centerIn: parent
            spacing: Theme.s3
            Icon {
                name: "circle-check"
                color: Theme.bgRaised
            }
            Label {
                text: host.current
                color: Theme.bgRaised
                wrapMode: Text.NoWrap
            }
            IconButton {
                id: closeButton
                label: qsTr("Dismiss")
                iconName: "x"
                ink: Theme.bgRaised
                onClicked: host.next()
            }
        }
    }
}
