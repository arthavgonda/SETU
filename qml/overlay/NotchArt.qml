import QtQuick
import Setu.Theme
import Ledger

Item {
    id: art
    property string source
    property real radius: Theme.px(6)
    property color tint: Theme.notchRaised
    readonly property bool ready: source.length > 0 && canvas.loaded === source
    implicitWidth: Theme.px(24)
    implicitHeight: Theme.px(24)
    Accessible.ignored: true

    Canvas {
        id: canvas
        anchors.fill: parent
        renderTarget: Canvas.Image
        property string loaded
        onImageLoaded: {
            if (isImageLoaded(art.source))
                loaded = art.source
            requestPaint()
        }
        Connections {
            target: art
            function onSourceChanged() {
                canvas.loaded = ""
                if (art.source.length > 0) {
                    if (canvas.isImageLoaded(art.source))
                        canvas.loaded = art.source
                    else
                        canvas.loadImage(art.source)
                }
                canvas.requestPaint()
            }
        }
        Component.onCompleted: {
            if (art.source.length > 0)
                loadImage(art.source)
        }
        onPaint: {
            const ctx = getContext("2d")
            ctx.reset()
            const r = Math.min(art.radius, width / 2, height / 2)
            ctx.beginPath()
            ctx.moveTo(r, 0)
            ctx.arcTo(width, 0, width, height, r)
            ctx.arcTo(width, height, 0, height, r)
            ctx.arcTo(0, height, 0, 0, r)
            ctx.arcTo(0, 0, width, 0, r)
            ctx.closePath()
            ctx.save()
            ctx.clip()
            if (art.ready) {
                ctx.drawImage(art.source, 0, 0, width, height)
            } else {
                ctx.fillStyle = art.tint
                ctx.fillRect(0, 0, width, height)
            }
            ctx.restore()
        }
    }
    Icon {
        anchors.centerIn: parent
        visible: !art.ready
        name: "music"
        size: Math.round(art.height * 0.55)
        color: Theme.notchInk
    }
}
