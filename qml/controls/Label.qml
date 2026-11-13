import QtQuick
import Setu.Theme

Text {
    property string role: "body"
    readonly property var spec: Type.role(role)
    color: Theme.textPrimary
    Accessible.name: text
    font.family: spec.family
    font.pixelSize: spec.size
    font.weight: spec.weight
    lineHeight: spec.line
    lineHeightMode: Text.FixedHeight
    textFormat: Text.PlainText
    wrapMode: Text.Wrap
    verticalAlignment: Text.AlignVCenter
    renderType: Text.NativeRendering
}
