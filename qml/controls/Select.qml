import QtQuick
import QtQuick.Templates as T
import QtQuick.Layouts
import Setu.Theme

ColumnLayout {
    id: root
    property string label
    property string helper
    property var options: []
    property var value
    property string emptyText: qsTr("None")
    property alias box: box
    readonly property int currentIndex: indexOf(value)
    readonly property string currentText: currentIndex >= 0 ? options[currentIndex].text : emptyText
    signal activated(var value)
    signal blocked(var value)
    spacing: Theme.s1

    function indexOf(v) {
        for (let i = 0; i < options.length; ++i)
            if (options[i].value === v)
                return i
        return -1
    }

    function forceActiveFocus() {
        box.forceActiveFocus()
    }

    Label {
        text: root.label
        role: "label"
        visible: text.length > 0
        Layout.fillWidth: true
    }

    T.ComboBox {
        id: box
        Layout.fillWidth: true
        implicitHeight: Theme.controlHeight
        model: root.options
        textRole: "text"
        valueRole: "value"
        currentIndex: root.currentIndex
        hoverEnabled: true
        focusPolicy: Qt.StrongFocus
        leftPadding: Theme.s3
        rightPadding: Theme.s3 + Theme.iconSize + Theme.s1
        Accessible.role: Accessible.ComboBox
        Accessible.name: root.label
        Accessible.description: root.currentText
        Accessible.onPressAction: box.popup.visible ? box.popup.close() : box.popup.open()
        onActivated: index => {
            const o = root.options[index]
            if (o.disabled === true) {
                box.currentIndex = Qt.binding(() => root.currentIndex)
                root.blocked(o.value)
                return
            }
            root.value = o.value
            root.activated(o.value)
        }

        property string typed: ""
        Timer {
            id: typeReset
            interval: 800
            onTriggered: box.typed = ""
        }
        Keys.onPressed: event => {
            if (event.text.length === 1 && event.text.trim().length === 1 && !(event.modifiers & (Qt.ControlModifier | Qt.AltModifier))) {
                box.typed += event.text.toLowerCase()
                typeReset.restart()
                const start = box.popup.visible ? box.highlightedIndex : box.currentIndex
                for (let k = 1; k <= root.options.length; ++k) {
                    const i = (start + k - (box.typed.length > 1 ? 1 : 0) + root.options.length) % root.options.length
                    if (String(root.options[i].text).toLowerCase().startsWith(box.typed)) {
                        if (box.popup.visible) {
                            for (let guard = 0; box.highlightedIndex < i && guard < root.options.length; ++guard)
                                box.incrementCurrentIndex()
                            for (let guard = 0; box.highlightedIndex > i && guard < root.options.length; ++guard)
                                box.decrementCurrentIndex()
                            list.positionViewAtIndex(i, ListView.Contain)
                        } else if (root.options[i].disabled === true) {
                            root.blocked(root.options[i].value)
                        } else {
                            root.value = root.options[i].value
                            root.activated(root.options[i].value)
                        }
                        break
                    }
                }
                event.accepted = true
            }
        }

        contentItem: Label {
            text: root.currentText
            elide: Text.ElideRight
            wrapMode: Text.NoWrap
            color: box.enabled ? Theme.textPrimary : Theme.textTertiary
        }
        indicator: Icon {
            x: box.width - width - Theme.s3
            anchors.verticalCenter: parent.verticalCenter
            name: "chevron-down"
            color: Theme.textSecondary
        }
        background: Rectangle {
            radius: Theme.radiusControl
            color: box.down ? Theme.bgPressed : box.hovered ? Theme.bgHover : Theme.bgRaised
            border.width: Theme.border
            border.color: box.enabled ? Theme.borderStrong : Theme.borderSubtle
            FocusRing {
                shown: box.visualFocus
            }
        }
        delegate: T.ItemDelegate {
            id: opt
            required property int index
            required property var modelData
            width: list.width
            implicitHeight: Math.max(Theme.hit + Theme.s2, optLabel.implicitHeight + Theme.s2 * 2)
            highlighted: box.highlightedIndex === index
            hoverEnabled: true
            leftPadding: Theme.s3
            readonly property bool unavailable: modelData.disabled === true
            Accessible.role: Accessible.ListItem
            Accessible.name: modelData.text
            Accessible.description: modelData.note || ""
            Accessible.selected: box.currentIndex === index
            Accessible.onPressAction: opt.clicked()
            contentItem: RowLayout {
                spacing: Theme.s2
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    Label {
                        id: optLabel
                        Layout.fillWidth: true
                        text: opt.modelData.text
                        color: opt.unavailable ? Theme.textTertiary : Theme.textPrimary
                        font.weight: box.currentIndex === opt.index ? Font.DemiBold : Font.Normal
                    }
                    Label {
                        Layout.fillWidth: true
                        visible: text.length > 0
                        text: opt.modelData.note || ""
                        role: "small"
                        color: opt.unavailable ? Theme.danger : Theme.textSecondary
                    }
                }
                Icon {
                    name: box.currentIndex === opt.index ? "check" : ""
                    color: Theme.accent
                }
            }
            background: Rectangle {
                color: opt.highlighted ? Theme.bgSelected : opt.hovered ? Theme.bgHover : "transparent"
                border.width: Theme.contrast && opt.highlighted ? Theme.border : 0
                border.color: Theme.accent
            }
        }
        popup: T.Popup {
            y: box.height + Theme.s1
            width: box.width
            implicitHeight: Math.min(list.contentHeight + 2, Theme.px(320))
            padding: 1
            contentItem: ListView {
                id: list
                clip: true
                model: box.delegateModel
                currentIndex: box.highlightedIndex
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: ScrollBar {}
            }
            background: Rectangle {
                radius: Theme.radiusPanel
                color: Theme.bgRaised
                border.width: Theme.border
                border.color: Theme.borderStrong
            }
        }
    }

    Label {
        Layout.fillWidth: true
        visible: root.helper.length > 0
        text: root.helper
        role: "small"
        color: Theme.textSecondary
    }
}
