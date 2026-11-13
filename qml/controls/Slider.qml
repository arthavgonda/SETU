import QtQuick
import QtQuick.Templates as T
import QtQuick.Layouts
import Setu.Theme

ColumnLayout {
    id: root
    property string label
    property alias from: slider.from
    property alias to: slider.to
    property alias stepSize: slider.stepSize
    property real value: 0
    property string lowText
    property string highText
    property var format: v => String(Math.round(v * 100) / 100)
    property alias slider: slider
    signal moved(real value)
    spacing: Theme.s1

    function forceActiveFocus() {
        slider.forceActiveFocus()
    }

    RowLayout {
        Layout.fillWidth: true
        Label {
            text: root.label
            role: "label"
            Layout.fillWidth: true
        }
        Label {
            text: root.format(slider.value)
            role: "mono"
            color: Theme.textSecondary
        }
    }

    T.Slider {
        id: slider
        Layout.fillWidth: true
        implicitHeight: Theme.hit
        value: root.value
        snapMode: T.Slider.SnapAlways
        focusPolicy: Qt.StrongFocus
        hoverEnabled: true
        live: true
        Accessible.role: Accessible.Slider
        Accessible.name: root.label
        Accessible.description: root.format(slider.value)
        onMoved: {
            root.value = value
            root.moved(value)
        }
        Keys.onPressed: event => {
            const step = (slider.stepSize > 0 ? slider.stepSize : (slider.to - slider.from) / 100) * 10
            if (event.key === Qt.Key_PageUp || event.key === Qt.Key_PageDown) {
                const v = Math.max(slider.from, Math.min(slider.to, slider.value + (event.key === Qt.Key_PageUp ? step : -step)))
                slider.value = v
                slider.moved()
                event.accepted = true
            } else if (event.key === Qt.Key_Home || event.key === Qt.Key_End) {
                slider.value = event.key === Qt.Key_Home ? slider.from : slider.to
                slider.moved()
                event.accepted = true
            }
        }
        background: Rectangle {
            x: slider.leftPadding
            y: slider.topPadding + slider.availableHeight / 2 - height / 2
            width: slider.availableWidth
            height: 4
            radius: 2
            color: Theme.bgSunken
            border.width: Theme.contrast ? 1 : 0
            border.color: Theme.borderStrong
            Rectangle {
                width: slider.visualPosition * parent.width
                height: parent.height
                radius: 2
                color: slider.enabled ? Theme.accent : Theme.textTertiary
            }
        }
        handle: Rectangle {
            x: slider.leftPadding + slider.visualPosition * (slider.availableWidth - width)
            y: slider.topPadding + slider.availableHeight / 2 - height / 2
            width: Theme.px(18)
            height: width
            radius: width / 2
            color: Theme.bgRaised
            border.width: 2
            border.color: slider.enabled ? Theme.accent : Theme.textTertiary
            FocusRing {
                shown: slider.visualFocus
                radiusBase: width / 2
            }
        }
    }

    RowLayout {
        Layout.fillWidth: true
        visible: root.lowText.length > 0 || root.highText.length > 0
        Label {
            text: root.lowText
            role: "small"
            color: Theme.textSecondary
            Layout.fillWidth: true
        }
        Label {
            text: root.highText
            role: "small"
            color: Theme.textSecondary
            horizontalAlignment: Text.AlignRight
            Layout.fillWidth: true
        }
    }
}
