import QtQuick
import Setu.Theme

Image {
    property string name
    property color color: Theme.textPrimary
    property int size: Theme.iconSize
    width: size
    height: size
    visible: name.length > 0
    source: name.length > 0 ? Theme.icon(name, color) : ""
    sourceSize.width: Math.round(size * Screen.devicePixelRatio)
    sourceSize.height: Math.round(size * Screen.devicePixelRatio)
    smooth: true
    asynchronous: false
    cache: true
    Accessible.ignored: true
}
