import QtQuick
import QtQuick.Templates as T
import Setu.Theme

Row {
    id: root
    property var tabs: []
    property string current
    signal activated(string id)
    spacing: Theme.s1
    Accessible.role: Accessible.PageTabList

    function moveTo(i) {
        const n = (i + tabs.length) % tabs.length
        activated(tabs[n].id)
        rep.itemAt(n).forceActiveFocus()
    }

    Repeater {
        id: rep
        model: root.tabs
        T.TabButton {
            id: tab
            required property int index
            required property var modelData
            readonly property bool on: modelData.id === root.current
            implicitHeight: Theme.controlHeight
            implicitWidth: tl.implicitWidth + Theme.s4 * 2
            hoverEnabled: true
            focusPolicy: on ? Qt.StrongFocus : Qt.ClickFocus
            activeFocusOnTab: on
            Accessible.role: Accessible.PageTab
            Accessible.name: modelData.title
            Accessible.selected: on
            Accessible.onPressAction: root.activated(modelData.id)
            onClicked: root.activated(modelData.id)
            Keys.onLeftPressed: root.moveTo(index - 1)
            Keys.onRightPressed: root.moveTo(index + 1)
            contentItem: Label {
                id: tl
                text: tab.modelData.title
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.NoWrap
                font.weight: tab.on ? Font.DemiBold : Font.Normal
            }
            background: Rectangle {
                color: tab.hovered ? Theme.bgHover : "transparent"
                radius: Theme.radiusControl
                Rectangle {
                    anchors.bottom: parent.bottom
                    width: parent.width
                    height: 2
                    visible: tab.on
                    color: Theme.accent
                }
                FocusRing {
                    shown: tab.visualFocus
                }
            }
        }
    }
}
