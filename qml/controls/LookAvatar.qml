import QtQuick
import Setu.Theme

Item {
    id: rp
    property string avatar: ""
    property string file: ""
    property color fill: Theme.lookTile
    readonly property string fileUrl: file.length > 0 ? (file.startsWith("file:") ? file : "file://" + file) : ""
    readonly property bool ready: file.length > 0 ? probe.status === Image.Ready : bundled.status === Image.Ready
    implicitWidth: 48
    implicitHeight: 48

    Rectangle {
        anchors.fill: parent
        radius: width / 2
        color: rp.fill
    }
    Image {
        id: bundled
        anchors.fill: parent
        visible: rp.file.length === 0
        source: rp.file.length === 0 && rp.avatar.length > 0 ? "qrc:/looks/avatars/" + rp.avatar + ".png" : ""
        sourceSize: Qt.size(Math.ceil(width * 2), Math.ceil(height * 2))
        fillMode: Image.PreserveAspectFit
        asynchronous: true
        smooth: true
        mipmap: true
        Accessible.ignored: true
    }
    Image {
        id: probe
        visible: false
        Accessible.ignored: true
        source: rp.fileUrl
        asynchronous: true
        onStatusChanged: if (status === Image.Ready) round.requestPaint()
    }
    Canvas {
        id: round
        anchors.fill: parent
        visible: rp.file.length > 0
        renderTarget: Canvas.Image
        onWidthChanged: requestPaint()
        Component.onCompleted: if (rp.fileUrl.length > 0) loadImage(rp.fileUrl)
        Connections {
            target: rp
            function onFileUrlChanged() {
                if (rp.fileUrl.length > 0)
                    round.loadImage(rp.fileUrl)
                round.requestPaint()
            }
        }
        onImageLoaded: requestPaint()
        onPaint: {
            const c = getContext("2d")
            c.reset()
            if (rp.fileUrl.length === 0 || !isImageLoaded(rp.fileUrl) || probe.implicitWidth <= 0)
                return
            const iw = probe.implicitWidth, ih = probe.implicitHeight
            const side = Math.min(iw, ih)
            const sx = (iw - side) / 2
            const sy = ih > iw ? (ih - side) * 0.2 : 0
            c.save()
            c.beginPath()
            c.arc(width / 2, height / 2, width / 2, 0, Math.PI * 2)
            c.closePath()
            c.clip()
            c.drawImage(rp.fileUrl, sx, sy, side, side, 0, 0, width, height)
            c.restore()
        }
    }
}
