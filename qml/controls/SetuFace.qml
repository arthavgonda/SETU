import QtQuick
import Setu.Core
import Setu.Theme

Rectangle {
    id: face
    property alias mood: eyes.mood
    property alias level: eyes.level
    implicitWidth: Theme.px(132)
    implicitHeight: Theme.px(64)
    radius: height / 2
    color: Theme.notchBg
    border.width: Theme.contrast ? 2 : (Theme.dark ? 1 : 0)
    border.color: Theme.contrast ? Theme.notchInk : Theme.borderStrong
    Accessible.role: Accessible.Graphic
    Accessible.name: qsTr("%1's face").arg(Brand.productName)
    Eyes {
        id: eyes
        anchors.centerIn: parent
        width: Math.round(face.width * 0.44)
        height: Math.round(face.height * 0.42)
    }
}
