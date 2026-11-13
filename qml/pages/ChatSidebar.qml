import QtQuick
import QtQuick.Layouts
import QtQuick.Templates as T
import Setu.Core
import Setu.Theme
import Ledger

FocusScope {
    id: root
    property var chats: []
    property int current: -1
    property string query
    property bool loading: true
    property int renaming: -1
    signal open(int id)
    signal fresh
    signal settings
    signal about
    signal collapse
    implicitWidth: Theme.sidebarWidth

    readonly property string firstName: Platform.userName.split(" ")[0]
    readonly property var rows: chats.map(c => Object.assign({ group: groupOf(c.updated) }, c))

    function groupOf(ms) {
        const d = new Date(ms)
        const start = new Date()
        start.setHours(0, 0, 0, 0)
        const day = 86400000
        if (ms >= start.getTime())
            return qsTr("Today")
        if (ms >= start.getTime() - day)
            return qsTr("Yesterday")
        if (ms >= start.getTime() - 7 * day)
            return qsTr("Previous 7 days")
        if (ms >= start.getTime() - 30 * day)
            return qsTr("Previous 30 days")
        return d.toLocaleDateString(Qt.locale(), "MMMM yyyy")
    }

    function reload() {
        Daemon.call("chat.list", { limit: 300, query: root.query }, (ok, r) => {
            root.loading = false
            if (ok)
                root.chats = r.items
        })
    }

    function upsert(c) {
        if (root.query.length > 0) {
            searchTimer.restart()
            return
        }
        let list = root.chats.filter(x => x.id !== c.id)
        list.push(c)
        list.sort((a, b) => b.updated - a.updated)
        root.chats = list
    }

    function remove(id) {
        Daemon.call("chat.delete", { id: id }, (ok, r, e) => {
            if (!ok)
                return
            root.chats = root.chats.filter(x => x.id !== id)
            if (root.current === id)
                root.fresh()
        })
    }

    function rename(id, title) {
        root.renaming = -1
        if (title.trim().length > 0)
            Daemon.call("chat.rename", { id: id, title: title.trim() })
    }

    function focusSearch() {
        search.forceActiveFocus()
    }

    Component.onCompleted: reload()

    Connections {
        target: Daemon
        function onReconnected() {
            root.reload()
        }
    }

    Subscription {
        topics: ["chat.changed"]
        onReceived: (topic, d) => {
            if (d.all) {
                root.chats = []
                root.fresh()
            } else if (d.deleted !== undefined && d.deleted !== null) {
                root.chats = root.chats.filter(x => x.id !== d.deleted)
            } else if (d.chat) {
                root.upsert(d.chat)
            }
        }
    }

    Timer {
        id: searchTimer
        interval: 220
        onTriggered: root.reload()
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.s2
        spacing: Theme.s1

        RowLayout {
            Layout.fillWidth: true
            Layout.topMargin: Theme.s1
            Layout.bottomMargin: Theme.s2
            Layout.leftMargin: Theme.s1
            spacing: Theme.s2
            SetuFace {
                Layout.preferredWidth: Theme.px(36)
                Layout.preferredHeight: Theme.px(20)
                mood: Daemon.mic.muted === true ? "sleepy" : Daemon.session.state === "capturing" ? "listen" : "idle"
                Accessible.ignored: true
            }
            Label {
                Layout.fillWidth: true
                text: Brand.productName
                role: "heading"
                Accessible.ignored: true
            }
            IconButton {
                label: qsTr("Close sidebar")
                iconName: "panel-left"
                onClicked: root.collapse()
            }
        }

        SideAction {
            Layout.fillWidth: true
            text: qsTr("New chat")
            iconName: "square-pen"
            shortcutText: Platform.shortcut("Ctrl+Shift+O")
            onClicked: root.fresh()
        }

        SearchField {
            id: search
            Layout.fillWidth: true
            Layout.topMargin: Theme.s1
            Layout.bottomMargin: Theme.s1
            placeholder: qsTr("Search chats")
            onTextChanged: {
                root.query = text
                searchTimer.restart()
            }
        }

        ListView {
            id: list
            Layout.fillWidth: true
            Layout.fillHeight: true
            model: root.rows
            clip: true
            spacing: 1
            boundsBehavior: Flickable.StopAtBounds
            activeFocusOnTab: true
            keyNavigationEnabled: true
            Accessible.role: Accessible.List
            Accessible.name: qsTr("Chats")
            ScrollBar.vertical: ScrollBar {}
            section.property: "group"
            section.criteria: ViewSection.FullString
            section.delegate: Label {
                required property string section
                width: list.width
                topPadding: Theme.s3
                bottomPadding: Theme.s1
                leftPadding: Theme.s2
                text: section
                role: "small"
                color: Theme.textTertiary
                font.weight: Font.DemiBold
                Accessible.role: Accessible.Heading
            }
            delegate: T.ItemDelegate {
                id: row
                required property var modelData
                required property int index
                readonly property bool selected: modelData.id === root.current
                readonly property bool editing: root.renaming === modelData.id
                width: list.width
                implicitHeight: Math.max(Theme.hit + Theme.s1, title.implicitHeight + Theme.s2 * 2)
                leftPadding: Theme.s2
                rightPadding: Theme.s1
                hoverEnabled: true
                focusPolicy: Qt.StrongFocus
                Accessible.name: modelData.title
                Accessible.role: Accessible.ListItem
                Accessible.onPressAction: row.clicked()
                onClicked: {
                    if (!row.editing)
                        root.open(modelData.id)
                }
                Keys.onDeletePressed: root.remove(modelData.id)
                Keys.onPressed: event => {
                    if (event.key === Qt.Key_F2) {
                        root.renaming = modelData.id
                        event.accepted = true
                    }
                }
                background: Rectangle {
                    radius: Theme.radiusPanel
                    color: row.selected ? Theme.bgSelected : (row.hovered || row.visualFocus ? Theme.bgHover : "transparent")
                    border.width: row.visualFocus ? Theme.focusWidth : 0
                    border.color: Theme.accent
                    Behavior on color {
                        ColorAnimation { duration: Motion.micro }
                    }
                }
                contentItem: RowLayout {
                    spacing: Theme.s1
                    Label {
                        id: title
                        Layout.fillWidth: true
                        visible: !row.editing
                        text: row.modelData.title
                        elide: Text.ElideRight
                        maximumLineCount: 1
                        color: Theme.textPrimary
                        font.weight: row.selected ? Font.DemiBold : Font.Normal
                    }
                    TextField {
                        id: editor
                        Layout.fillWidth: true
                        visible: row.editing
                        text: row.modelData.title
                        onVisibleChanged: if (visible) Qt.callLater(() => editor.field.forceActiveFocus())
                        onAccepted: root.rename(row.modelData.id, editor.text)
                        Keys.onEscapePressed: root.renaming = -1
                    }
                    IconButton {
                        id: more
                        visible: !row.editing && (row.hovered || row.selected || row.visualFocus || menu.visible)
                        label: qsTr("Options for %1").arg(row.modelData.title)
                        iconName: "ellipsis"
                        implicitWidth: Theme.hit
                        implicitHeight: Theme.hit
                        onClicked: menu.popup(more, 0, more.height)
                    }
                }
                Menu {
                    id: menu
                    MenuItem {
                        text: qsTr("Rename")
                        iconName: "pencil"
                        onTriggered: root.renaming = row.modelData.id
                    }
                    MenuItem {
                        text: qsTr("Delete")
                        iconName: "trash-2"
                        onTriggered: root.remove(row.modelData.id)
                    }
                }
            }

            Label {
                anchors.centerIn: parent
                width: parent.width - Theme.s4
                visible: !root.loading && root.rows.length === 0
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
                role: "small"
                color: Theme.textTertiary
                text: root.query.length > 0 ? qsTr("No chats match.") : qsTr("Your chats will appear here.")
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: Theme.border
            color: Theme.borderSubtle
        }

        T.ItemDelegate {
            id: profile
            Layout.fillWidth: true
            implicitHeight: Math.max(Theme.rowHeight, profileRow.implicitHeight + Theme.s2 * 2)
            leftPadding: Theme.s2
            rightPadding: Theme.s2
            hoverEnabled: true
            focusPolicy: Qt.StrongFocus
            Accessible.name: qsTr("%1, account menu").arg(Platform.userName)
            Accessible.role: Accessible.ButtonMenu
            Accessible.onPressAction: profile.clicked()
            onClicked: profileMenu.popup(profile, 0, -profileMenu.implicitHeight - Theme.s1)
            background: Rectangle {
                radius: Theme.radiusPanel
                color: profile.hovered || profile.visualFocus || profileMenu.visible ? Theme.bgHover : "transparent"
                border.width: profile.visualFocus ? Theme.focusWidth : 0
                border.color: Theme.accent
            }
            contentItem: RowLayout {
                id: profileRow
                spacing: Theme.s2
                Avatar {
                    Layout.preferredWidth: Theme.px(32)
                    Layout.preferredHeight: Theme.px(32)
                    name: Platform.userName
                    source: Platform.userAvatar
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    Label {
                        Layout.fillWidth: true
                        text: Platform.userName
                        elide: Text.ElideRight
                        font.weight: Font.DemiBold
                    }
                    Label {
                        Layout.fillWidth: true
                        visible: Platform.userLogin.length > 0 && Platform.userLogin.toLowerCase() !== Platform.userName.toLowerCase()
                        text: Platform.userLogin
                        role: "small"
                        color: Theme.textTertiary
                        elide: Text.ElideRight
                    }
                }
                Icon {
                    name: "chevron-up"
                    color: Theme.textTertiary
                }
            }
            Menu {
                id: profileMenu
                MenuItem {
                    text: qsTr("Settings")
                    iconName: "settings"
                    shortcutText: Platform.shortcut("Ctrl+,")
                    onTriggered: root.settings()
                }
                MenuItem {
                    text: Daemon.mic.muted === true ? qsTr("Turn the microphone on") : qsTr("Mute microphone")
                    iconName: Daemon.mic.muted === true ? "mic" : "mic-off"
                    onTriggered: Daemon.call("mic.setMuted", { muted: Daemon.mic.muted !== true })
                }
                MenuItem {
                    text: qsTr("About %1").arg(Brand.productName)
                    iconName: "info"
                    onTriggered: root.about()
                }
            }
        }
    }

    component SideAction: T.ItemDelegate {
        id: action
        property string iconName
        property string shortcutText
        implicitHeight: Math.max(Theme.hit + Theme.s1, label.implicitHeight + Theme.s2 * 2)
        leftPadding: Theme.s2
        rightPadding: Theme.s2
        hoverEnabled: true
        focusPolicy: Qt.StrongFocus
        Accessible.name: text
        Accessible.role: Accessible.Button
        background: Rectangle {
            radius: Theme.radiusPanel
            color: action.down ? Theme.bgPressed : (action.hovered || action.visualFocus ? Theme.bgHover : "transparent")
            border.width: action.visualFocus ? Theme.focusWidth : 0
            border.color: Theme.accent
        }
        contentItem: RowLayout {
            spacing: Theme.s2
            Icon {
                name: action.iconName
                color: Theme.textPrimary
            }
            Label {
                id: label
                Layout.fillWidth: true
                text: action.text
                font.weight: Font.DemiBold
            }
            Label {
                visible: action.hovered && action.shortcutText.length > 0
                text: action.shortcutText
                role: "small"
                color: Theme.textTertiary
            }
        }
    }
}
