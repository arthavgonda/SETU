import QtQuick
import Setu.Theme
import Setu.ShellHost
import Ledger

Window {
    id: win
    property var elements: []
    readonly property rect area: OverlayHost.virtualGeometry()
    visible: true
    color: "transparent"
    x: area.x
    y: area.y
    width: area.width
    height: area.height
    title: qsTr("Numbers")
    Component.onCompleted: OverlayHost.prepare(win, true)
    Repeater {
        model: win.elements
        Rectangle {
            required property var modelData
            readonly property int bw: Math.max(Theme.px(20), num.implicitWidth + 8)
            readonly property int bh: num.implicitHeight + 2
            x: Math.max(0, Math.min(win.width - bw, modelData.x - win.area.x - (modelData.x - win.area.x > bw + 2 ? bw + 2 : 0)))
            y: Math.max(0, Math.min(win.height - bh, modelData.y - win.area.y + (modelData.x - win.area.x > bw + 2 ? 0 : -bh - 2)))
            width: bw
            height: bh
            radius: Theme.radiusControl
            color: Theme.accent
            border.width: 1
            border.color: Theme.textOnAccent
            Accessible.role: Accessible.StaticText
            Accessible.name: qsTr("%1, %2").arg(modelData.n).arg(modelData.label)
            Text {
                id: num
                anchors.centerIn: parent
                text: parent.modelData.n
                color: Theme.textOnAccent
                font.family: Type.mono
                font.pixelSize: 12
                font.weight: Font.DemiBold
            }
        }
    }
}
