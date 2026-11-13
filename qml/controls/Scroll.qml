import QtQuick
import QtQuick.Window
import Setu.Theme

Flickable {
    id: flick
    readonly property var hostWindow: Window.window
    default property alias content: holder.data
    property alias spacing: holder.spacing
    property int gutter: Theme.s6
    contentWidth: width
    contentHeight: holder.implicitHeight + gutter * 2
    clip: true
    boundsBehavior: Flickable.StopAtBounds
    flickableDirection: Flickable.VerticalFlick
    ScrollBar.vertical: ScrollBar {}
    Column {
        id: holder
        x: flick.gutter
        y: flick.gutter
        width: flick.width - flick.gutter * 2
        spacing: Theme.s5
    }
    function page(dir) {
        contentY = Math.max(0, Math.min(Math.max(0, contentHeight - height), contentY + dir * height * 0.9))
    }
    Keys.onPressed: event => {
        if (event.key === Qt.Key_PageDown || event.key === Qt.Key_PageUp) {
            flick.page(event.key === Qt.Key_PageDown ? 1 : -1)
            event.accepted = true
        }
    }
    function ensureVisible(item) {
        if (!item || height <= 0)
            return
        const p = item.mapToItem(holder, 0, 0)
        const top = p.y + flick.gutter
        const bottom = top + item.height
        if (top < contentY)
            contentY = Math.max(0, top - Theme.s4)
        else if (bottom > contentY + height)
            contentY = Math.max(0, Math.min(contentHeight - height, top - Theme.s4, bottom - height + Theme.s4))
    }
    Connections {
        target: flick.hostWindow
        function onActiveFocusItemChanged() {
            const f = flick.hostWindow ? flick.hostWindow.activeFocusItem : null
            let p = f
            while (p && p !== holder)
                p = p.parent
            if (p === holder)
                flick.ensureVisible(f)
        }
    }
}
