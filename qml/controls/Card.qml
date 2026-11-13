import QtQuick
import QtQuick.Layouts
import Setu.Theme

Rectangle {
    id: card
    default property alias content: inner.data
    property bool highlighted: false
    property int pad: Theme.s4
    implicitHeight: inner.implicitHeight + pad * 2
    implicitWidth: inner.implicitWidth + pad * 2
    radius: Theme.radiusOverlay
    color: Theme.bgRaised
    border.width: highlighted ? 2 : Theme.border
    border.color: highlighted ? Theme.accent : Theme.borderSubtle
    Behavior on border.color {
        ColorAnimation { duration: Motion.standard }
    }
    Behavior on implicitHeight {
        enabled: !Motion.reduced
        NumberAnimation { duration: Motion.spring; easing.type: Easing.OutBack; easing.overshoot: Motion.overshoot * 0.6 }
    }
    ColumnLayout {
        id: inner
        anchors.fill: parent
        anchors.margins: card.pad
        spacing: Theme.s3
    }
}
