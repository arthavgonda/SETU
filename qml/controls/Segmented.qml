import QtQuick
import QtQuick.Templates as T
import QtQuick.Layouts
import Setu.Theme

ColumnLayout {
    id: root
    property string label
    property var options: []
    property var value
    signal activated(var value)
    spacing: Theme.s1

    function select(i) {
        if (i < 0 || i >= options.length)
            return
        rep.itemAt(i).forceActiveFocus()
        value = options[i].value
        activated(value)
    }

    Label {
        text: root.label
        role: "label"
        visible: text.length > 0
    }
    Flow {
        id: row
        Layout.fillWidth: true
        spacing: 0
        Accessible.role: Accessible.Grouping
        Accessible.name: root.label
        Repeater {
            id: rep
            model: root.options
            T.AbstractButton {
                id: seg
                required property int index
                required property var modelData
                readonly property bool on: root.value === modelData.value
                implicitHeight: Theme.controlHeight
                implicitWidth: Math.max(Theme.px(72), segLabel.implicitWidth + Theme.s4 * 2)
                checkable: false
                hoverEnabled: true
                focusPolicy: on ? Qt.StrongFocus : Qt.ClickFocus
                activeFocusOnTab: on
                Accessible.role: Accessible.RadioButton
                Accessible.name: modelData.text
                Accessible.checkable: true
                Accessible.checked: on
                Accessible.onPressAction: root.select(index)
                onClicked: root.select(index)
                Keys.onLeftPressed: root.select(index - 1)
                Keys.onRightPressed: root.select(index + 1)
                Keys.onUpPressed: root.select(index - 1)
                Keys.onDownPressed: root.select(index + 1)
                contentItem: Label {
                    id: segLabel
                    text: seg.modelData.text
                    role: "label"
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.NoWrap
                    color: seg.on ? Theme.textOnAccent : Theme.textPrimary
                    font.weight: seg.on ? Font.DemiBold : Font.Medium
                }
                background: Rectangle {
                    color: seg.on ? Theme.accent : seg.down ? Theme.bgPressed : seg.hovered ? Theme.bgHover : Theme.bgRaised
                    border.width: Theme.border
                    border.color: seg.on ? Theme.accent : Theme.borderStrong
                    radius: Theme.radiusControl
                    FocusRing {
                        shown: seg.visualFocus
                    }
                }
            }
        }
    }
}
