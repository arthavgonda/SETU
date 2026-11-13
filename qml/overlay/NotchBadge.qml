import QtQuick
import Setu.Theme

Rectangle {
    id: badge
    property string name: ""
    readonly property var hues: [0.58, 0.02, 0.33, 0.78, 0.12, 0.5, 0.92, 0.66]
    readonly property real hue: {
        let h = 0
        for (let i = 0; i < name.length; ++i)
            h = (h * 31 + name.charCodeAt(i)) % 997
        return hues[h % hues.length]
    }
    implicitWidth: Theme.px(28)
    implicitHeight: implicitWidth
    radius: Math.round(width * 0.28)
    gradient: Gradient {
        GradientStop { position: 0; color: Qt.hsla(badge.hue, 0.72, 0.62, 1) }
        GradientStop { position: 1; color: Qt.hsla(badge.hue, 0.78, 0.44, 1) }
    }
    Accessible.ignored: true
    Text {
        anchors.centerIn: parent
        text: badge.name.length > 0 ? badge.name.trim().charAt(0).toUpperCase() : ""
        color: Theme.notchInk
        font.family: Type.sans
        font.pixelSize: Math.round(badge.width * 0.5)
        font.weight: Font.Bold
        renderType: Text.NativeRendering
    }
}
