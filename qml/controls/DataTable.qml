import QtQuick
import QtQuick.Templates as T
import QtQuick.Layouts
import Setu.Theme

FocusScope {
    id: root
    property var columns: []
    property alias model: list.model
    property string label
    property string sortKey
    property bool sortDesc: true
    property int currentColumn: 0
    property alias currentIndex: list.currentIndex
    property alias view: list
    property var widths: []
    property var cellText: (row, key) => String(row[key] ?? "")
    property var selected: ({})
    property bool selectable: true
    signal sortRequested(string key, bool desc)
    signal activated(int row)
    signal nearEnd
    signal selectionChanged

    readonly property int totalWidth: {
        let w = 0
        for (let i = 0; i < columns.length; ++i)
            w += colWidth(i)
        return w + (selectable ? Theme.px(40) : 0)
    }

    function colWidth(i) {
        return widths[i] !== undefined ? widths[i] : Theme.px(columns[i].width || 140)
    }

    function resize(i, delta) {
        let w = widths.slice()
        while (w.length < columns.length)
            w.push(undefined)
        w[i] = Math.max(Theme.px(60), colWidth(i) + delta)
        widths = w
    }

    function toggle(id) {
        let s = Object.assign({}, selected)
        if (s[id])
            delete s[id]
        else
            s[id] = true
        selected = s
        selectionChanged()
    }

    function clearSelection() {
        selected = ({})
        selectionChanged()
    }

    function rowIdAt(i) {
        const m = list.model
        const row = m && typeof m.get === "function" ? m.get(i) : (m ? m[i] : null)
        return String(row && row.id !== undefined ? row.id : i)
    }

    function selectedIds() {
        return Object.keys(selected)
    }

    Flickable {
        id: hscroll
        anchors.fill: parent
        contentWidth: Math.max(width, root.totalWidth)
        contentHeight: height
        flickableDirection: Flickable.HorizontalFlick
        boundsBehavior: Flickable.StopAtBounds
        clip: true
        interactive: contentWidth > width
        ScrollBar.horizontal: ScrollBar {}

        ListView {
            id: list
            width: hscroll.contentWidth
            height: hscroll.height
            clip: true
            focus: true
            reuseItems: true
            cacheBuffer: Theme.rowHeight * 20
            boundsBehavior: Flickable.StopAtBounds
            headerPositioning: ListView.OverlayHeader
            keyNavigationEnabled: true
            activeFocusOnTab: true
            Accessible.role: Accessible.Table
            Accessible.name: root.label
            ScrollBar.vertical: ScrollBar {}
            onContentYChanged: if (contentHeight - contentY - height < Theme.rowHeight * 30) root.nearEnd()
            Keys.onLeftPressed: root.currentColumn = Math.max(0, root.currentColumn - 1)
            Keys.onRightPressed: root.currentColumn = Math.min(root.columns.length - 1, root.currentColumn + 1)
            Keys.onReturnPressed: root.activated(currentIndex)
            Keys.onEnterPressed: root.activated(currentIndex)
            Keys.onSpacePressed: if (root.selectable && currentIndex >= 0) root.toggle(root.rowIdAt(currentIndex))
            Keys.onPressed: event => {
                if (event.key === Qt.Key_Home) {
                    currentIndex = 0
                    event.accepted = true
                } else if (event.key === Qt.Key_End) {
                    currentIndex = count - 1
                    event.accepted = true
                } else if (event.key === Qt.Key_PageDown) {
                    currentIndex = Math.min(count - 1, currentIndex + Math.floor(height / Theme.rowHeight))
                    event.accepted = true
                } else if (event.key === Qt.Key_PageUp) {
                    currentIndex = Math.max(0, currentIndex - Math.floor(height / Theme.rowHeight))
                    event.accepted = true
                }
            }

            header: Rectangle {
                z: 3
                width: list.width
                height: Theme.controlHeight
                color: Theme.bgSunken
                Accessible.role: Accessible.Row
                Rectangle {
                    anchors.bottom: parent.bottom
                    width: parent.width
                    height: Theme.border
                    color: Theme.borderStrong
                }
                Row {
                    height: parent.height
                    Item {
                        visible: root.selectable
                        width: Theme.px(40)
                        height: parent.height
                    }
                    Repeater {
                        model: root.columns
                        T.AbstractButton {
                            id: head
                            required property int index
                            required property var modelData
                            width: root.colWidth(index)
                            height: parent.height
                            hoverEnabled: true
                            focusPolicy: modelData.sortable === false ? Qt.NoFocus : Qt.StrongFocus
                            Accessible.role: Accessible.ColumnHeader
                            Accessible.onPressAction: head.clicked()
                            Accessible.name: modelData.title + (root.sortKey === modelData.key ? (root.sortDesc ? qsTr(", sorted descending") : qsTr(", sorted ascending")) : "")
                            onClicked: if (modelData.sortable !== false) root.sortRequested(modelData.key, root.sortKey === modelData.key ? !root.sortDesc : true)
                            Keys.onPressed: event => {
                                if (event.modifiers & Qt.AltModifier && (event.key === Qt.Key_Left || event.key === Qt.Key_Right)) {
                                    root.resize(index, event.key === Qt.Key_Right ? Theme.px(16) : -Theme.px(16))
                                    event.accepted = true
                                }
                            }
                            contentItem: RowLayout {
                                spacing: Theme.s1
                                Label {
                                    Layout.fillWidth: true
                                    text: head.modelData.title
                                    role: "label"
                                    elide: Text.ElideRight
                                    wrapMode: Text.NoWrap
                                }
                                Icon {
                                    name: root.sortKey === head.modelData.key ? (root.sortDesc ? "chevron-down" : "chevron-up") : ""
                                    size: Theme.px(16)
                                }
                            }
                            leftPadding: Theme.s3
                            rightPadding: Theme.s3
                            background: Rectangle {
                                color: head.hovered ? Theme.bgHover : "transparent"
                                FocusRing {
                                    shown: head.visualFocus
                                    anchors.margins: 0
                                }
                            }
                            MouseArea {
                                width: 8
                                height: parent.height
                                anchors.right: parent.right
                                cursorShape: Qt.SplitHCursor
                                property real startX
                                onPressed: mouse => startX = mouse.x
                                onPositionChanged: mouse => {
                                    if (pressed)
                                        root.resize(head.index, mouse.x - startX)
                                }
                            }
                        }
                    }
                }
            }

            delegate: T.ItemDelegate {
                id: rowItem
                required property int index
                required property var model
                readonly property var rowData: model.item !== undefined ? model.item : model.modelData
                readonly property string rowId: String(rowData ? rowData.id : index)
                readonly property bool isCurrent: ListView.isCurrentItem
                width: list.width
                implicitHeight: Theme.rowHeight
                hoverEnabled: true
                focusPolicy: Qt.StrongFocus
                padding: 0
                Accessible.role: Accessible.Row
                Accessible.name: root.columns.map(c => c.title + " " + root.cellText(rowData, c.key)).join(", ")
                Accessible.selected: root.selected[rowId] === true
                Accessible.onPressAction: rowItem.clicked()
                onClicked: {
                    list.currentIndex = index
                    root.activated(index)
                }
                contentItem: Row {
                    Item {
                        visible: root.selectable
                        width: Theme.px(40)
                        height: rowItem.height
                        Checkbox {
                            anchors.centerIn: parent
                            checked: root.selected[rowItem.rowId] === true
                            onToggled: root.toggle(rowItem.rowId)
                            focusPolicy: Qt.NoFocus
                            Accessible.name: qsTr("Select row")
                            implicitWidth: Theme.hit
                        }
                    }
                    Repeater {
                        model: root.columns
                        Item {
                            required property int index
                            required property var modelData
                            width: root.colWidth(index)
                            height: rowItem.height
                            Accessible.role: Accessible.Cell
                            Accessible.name: cellLabel.text
                            Label {
                                id: cellLabel
                                anchors.fill: parent
                                anchors.leftMargin: Theme.s3
                                anchors.rightMargin: Theme.s3
                                text: root.cellText(rowItem.rowData, parent.modelData.key)
                                role: parent.modelData.mono ? "mono" : "body"
                                elide: Text.ElideRight
                                wrapMode: Text.NoWrap
                                color: parent.modelData.secondary ? Theme.textSecondary : Theme.textPrimary
                            }
                            Rectangle {
                                anchors.fill: parent
                                anchors.margins: 2
                                color: "transparent"
                                border.width: 2
                                border.color: Theme.accent
                                radius: Theme.radiusControl
                                visible: rowItem.isCurrent && list.activeFocus && root.currentColumn === index
                            }
                        }
                    }
                }
                background: Rectangle {
                    color: root.selected[rowItem.rowId] ? Theme.bgSelected : rowItem.isCurrent && list.activeFocus ? Theme.bgHover : rowItem.hovered ? Theme.bgHover : "transparent"
                    Rectangle {
                        anchors.bottom: parent.bottom
                        width: parent.width
                        height: 1
                        color: Theme.borderSubtle
                    }
                }
            }
        }
    }
}
