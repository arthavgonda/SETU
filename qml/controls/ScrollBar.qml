import QtQuick
import QtQuick.Templates as T
import Setu.Theme

T.ScrollBar {
    id: control
    implicitWidth: orientation === Qt.Vertical ? 10 : 0
    implicitHeight: orientation === Qt.Horizontal ? 10 : 0
    padding: 2
    minimumSize: 0.08
    visible: policy !== T.ScrollBar.AlwaysOff && size < 1
    contentItem: Rectangle {
        implicitWidth: 6
        implicitHeight: 6
        radius: Theme.radiusControl
        color: control.pressed ? Theme.textSecondary : Theme.borderStrong
        opacity: control.active || control.hovered || Theme.contrast ? 1 : 0.55
    }
    Accessible.ignored: true
}
