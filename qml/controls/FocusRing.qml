import QtQuick
import Setu.Theme

Rectangle {
    property Item target: parent
    property bool shown: false
    property real radiusBase: Theme.radiusControl
    anchors.fill: target
    anchors.margins: -(Theme.focusGap + Theme.focusWidth)
    radius: radiusBase + Theme.focusGap + Theme.focusWidth
    color: "transparent"
    border.width: Theme.focusWidth
    border.color: Theme.accent
    visible: shown
    Accessible.ignored: true
}
