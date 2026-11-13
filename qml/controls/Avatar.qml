import QtQuick
import Setu.Theme

Rectangle {
    id: avatar
    property string name
    property string source
    readonly property string initials: {
        const parts = name.trim().split(/\s+/).filter(p => p.length > 0)
        if (parts.length === 0)
            return "?"
        const first = parts[0].charAt(0)
        const last = parts.length > 1 ? parts[parts.length - 1].charAt(0) : ""
        return String(first + last).toUpperCase()
    }
    implicitWidth: Theme.px(32)
    implicitHeight: Theme.px(32)
    radius: width / 2
    color: picture.status === Image.Ready ? "transparent" : Theme.accent
    Accessible.role: Accessible.Graphic
    Accessible.name: name

    Text {
        anchors.centerIn: parent
        visible: picture.status !== Image.Ready
        text: avatar.initials
        color: Theme.textOnAccent
        font.family: Type.sans
        font.pixelSize: Math.round(avatar.height * 0.4)
        font.weight: Font.DemiBold
        Accessible.ignored: true
    }
    Image {
        id: picture
        anchors.fill: parent
        source: avatar.source
        sourceSize: Qt.size(width * 2, height * 2)
        fillMode: Image.PreserveAspectCrop
        smooth: true
        visible: status === Image.Ready
        Accessible.ignored: true
    }
}
