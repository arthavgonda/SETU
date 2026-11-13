import QtQuick
import QtQuick.Templates as T
import Setu.Theme

T.Menu {
    id: menu
    implicitWidth: Math.max(Theme.px(220), contentItem.implicitWidth + leftPadding + rightPadding)
    implicitHeight: contentItem.implicitHeight + topPadding + bottomPadding
    padding: Theme.s1
    margins: 0
    delegate: MenuItem {}
    contentItem: ListView {
        implicitHeight: contentHeight
        implicitWidth: 220
        model: menu.contentModel
        interactive: Window.window ? contentHeight + menu.topPadding + menu.bottomPadding > Window.window.height : false
        clip: true
        currentIndex: menu.currentIndex
        ScrollBar.vertical: ScrollBar {}
    }
    background: Rectangle {
        radius: Theme.radiusPanel
        color: Theme.bgRaised
        border.width: Theme.border
        border.color: Theme.borderStrong
    }
}
