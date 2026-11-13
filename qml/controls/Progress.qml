import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme

ColumnLayout {
    id: root
    property string label
    property real bytes: 0
    property real total: 0
    property real rate: 0
    property int etaS: -1
    property string status
    property bool indeterminate: total <= 0
    property string activity
    readonly property real fraction: total > 0 ? Math.min(1, bytes / total) : 0
    readonly property int percent: Math.floor(fraction * 100)
    property int lastAnnounced: -1
    spacing: Theme.s1
    Accessible.role: Accessible.ProgressBar
    Accessible.name: label
    Accessible.description: indeterminate ? activity : qsTr("%1 percent").arg(percent)

    onPercentChanged: {
        const step = Math.floor(percent / 25) * 25
        if (!indeterminate && step !== lastAnnounced && step > 0) {
            lastAnnounced = step
            Announcer.announce(root, qsTr("%1, %2 percent").arg(label).arg(step))
        }
    }

    RowLayout {
        Layout.fillWidth: true
        visible: root.label.length > 0
        Label {
            Layout.fillWidth: true
            text: root.label
            role: "label"
        }
        Label {
            visible: !root.indeterminate
            text: root.percent + "%"
            role: "mono"
        }
    }
    Rectangle {
        Layout.fillWidth: true
        Layout.preferredHeight: 6
        radius: 3
        color: Theme.bgSunken
        border.width: Theme.contrast ? 1 : 0
        border.color: Theme.borderStrong
        clip: true
        Rectangle {
            height: parent.height
            radius: 3
            color: Theme.accent
            width: root.indeterminate ? parent.width * 0.3 : parent.width * root.fraction
            x: root.indeterminate ? sweep.value * (parent.width * 0.7) : 0
        }
        NumberAnimation {
            id: sweep
            property real value: 0
            target: sweep
            property: "value"
            from: 0
            to: 1
            duration: 1400
            loops: Animation.Infinite
            running: root.indeterminate && root.visible && !Motion.reduced
        }
    }
    Label {
        Layout.fillWidth: true
        role: "mono"
        color: Theme.textSecondary
        text: root.indeterminate ? root.activity : [Platform.formatBytes(root.bytes) + " / " + Platform.formatBytes(root.total), root.rate > 0 ? Platform.formatBytes(root.rate) + "/s" : "", root.etaS >= 0 ? qsTr("%1 left").arg(Platform.formatDuration(root.etaS)) : ""].filter(s => s.length > 0).join("  ·  ")
        visible: text.length > 0
    }
}
