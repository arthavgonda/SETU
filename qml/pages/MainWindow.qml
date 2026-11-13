import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

Window {
    id: win
    readonly property string kind: "main"
    property string initialPage: "home"
    property string page: "chat"
    property string tab: ""
    property string pendingFocus: ""
    property bool sidebarOpen: width >= Theme.px(760)
    readonly property var settingsTabs: ["voice", "models", "permissions", "privacy", "accessibility", "general", "apps", "plugins"]
    readonly property string blocking: Daemon.status === "mismatch" || Daemon.status === "refused" || Daemon.status === "stopped" ? Daemon.status : ""
    readonly property Item loader: page === "settings" ? settingsLoader : (page === "about" ? aboutLoader : chat)

    width: 1080
    height: 720
    minimumWidth: 640
    minimumHeight: 520
    visible: true
    color: Theme.bgBase
    title: page === "settings" ? qsTr("Settings · %1").arg(Brand.productName) : page === "about" ? qsTr("About · %1").arg(Brand.productName) : (chat.chatTitle.length > 0 ? qsTr("%1 · %2").arg(chat.chatTitle).arg(Brand.productName) : Brand.productName)

    property bool timedFirst: false
    property real switchStarted: 0
    onFrameSwapped: {
        if (!Platform.overrides.timing)
            return
        if (!timedFirst) {
            timedFirst = true
            console.warn("setu-timing first-frame-ms", Platform.processMs())
        }
        if (switchStarted > 0) {
            console.warn("setu-timing page-switch-ms", Date.now() - switchStarted)
            switchStarted = 0
        }
    }

    Component.onCompleted: go(initialPage)

    signal wizardRequested

    function go(target) {
        let p = String(target || "home")
        if (p === "wizard") {
            wizardRequested()
            return
        }
        if (p.startsWith("chat/")) {
            const id = Number(p.slice(5))
            if (!isNaN(id))
                Qt.callLater(() => win.openChat(id))
            p = "chat"
        }
        if (p.startsWith("settings/"))
            p = p.slice(9)
        if (p.startsWith("skills/"))
            p = p.slice(7)
        let t = ""
        if (p === "settings" || p.length === 0) {
            p = "settings"
            t = tab.length > 0 && settingsTabs.includes(tab) ? tab : "voice"
        } else if (settingsTabs.includes(p)) {
            t = p
            p = "settings"
        } else if (p === "skills") {
            t = "apps"
            p = "settings"
        } else if (p !== "about") {
            p = "chat"
        }
        if (Platform.overrides.timing && p !== page)
            switchStarted = Date.now()
        tab = t
        page = p
        const section = settingsLoader.item as SectionPage
        if (section && t.length > 0)
            section.show(t)
        toFocus.restart()
    }

    function jump(target, key) {
        pendingFocus = key || ""
        go(target)
    }

    function toast(text) {
        toasts.show(text)
    }

    function newChat() {
        sidebar.current = -1
        chat.reset()
        go("chat")
    }

    function openChat(id) {
        sidebar.current = id
        chat.load(id)
        go("chat")
        if (width < Theme.px(760))
            sidebarOpen = false
    }

    Timer {
        id: toFocus
        interval: 0
        onTriggered: {
            if (win.pendingFocus.length > 0 && settingsLoader.item) {
                const it = win.findChild(settingsLoader.item, "setting:" + win.pendingFocus)
                win.pendingFocus = ""
                if (it) {
                    it.forceActiveFocus()
                    return
                }
            }
            if (win.page === "chat")
                chat.forceActiveFocus()
        }
    }

    function findChild(item, name) {
        if (!item)
            return null
        if (item.objectName === name)
            return item
        const kids = item.children || []
        for (let i = 0; i < kids.length; ++i) {
            const r = findChild(kids[i], name)
            if (r)
                return r
        }
        if (item.contentItem && item.contentItem !== item)
            return findChild(item.contentItem, name)
        return null
    }

    Shortcut {
        sequences: ["Ctrl+,"]
        onActivated: win.go("settings")
    }
    Shortcut {
        sequences: ["Ctrl+Shift+O"]
        onActivated: win.newChat()
    }
    Shortcut {
        sequences: ["Ctrl+K"]
        onActivated: {
            win.sidebarOpen = true
            sidebar.focusSearch()
        }
    }
    Shortcut {
        sequences: ["Ctrl+Shift+S"]
        onActivated: win.sidebarOpen = !win.sidebarOpen
    }
    Shortcut {
        sequences: [StandardKey.Find]
        onActivated: {
            if (win.page !== "settings") {
                win.sidebarOpen = true
                sidebar.focusSearch()
                return
            }
            const f = win.findChild(settingsLoader.item, "settings-search")
            if (f)
                f.forceActiveFocus()
        }
    }
    Shortcut {
        sequence: "Escape"
        enabled: win.page !== "chat"
        onActivated: win.go("chat")
    }
    Shortcut {
        sequence: "F6"
        onActivated: {
            if (sidebar.activeFocus)
                win.loader.forceActiveFocus()
            else {
                win.sidebarOpen = true
                sidebar.forceActiveFocus()
            }
        }
    }

    RowLayout {
        anchors.fill: parent
        LayoutMirroring.enabled: Platform.rightToLeft
        LayoutMirroring.childrenInherit: true
        spacing: 0

        Rectangle {
            id: rail
            Layout.fillHeight: true
            Layout.preferredWidth: win.sidebarOpen ? Theme.sidebarWidth + Theme.s5 : 0
            visible: Layout.preferredWidth > 1
            clip: true
            color: Theme.bgSunken
            Accessible.role: Accessible.Pane
            Accessible.name: qsTr("Chats")
            Behavior on Layout.preferredWidth {
                enabled: !Motion.reduced
                NumberAnimation { duration: Motion.standard; easing.type: Easing.OutCubic }
            }
            ChatSidebar {
                id: sidebar
                width: Theme.sidebarWidth + Theme.s5
                height: parent.height
                onOpen: id => win.openChat(id)
                onFresh: win.newChat()
                onSettings: win.go("settings")
                onAbout: win.go("about")
                onCollapse: win.sidebarOpen = false
            }
            Rectangle {
                anchors.right: parent.right
                width: Theme.border
                height: parent.height
                color: Theme.borderSubtle
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

            RowLayout {
                Layout.fillWidth: true
                Layout.leftMargin: Theme.s3
                Layout.rightMargin: Theme.s3
                Layout.topMargin: Theme.s2
                visible: !win.sidebarOpen || win.page !== "chat"
                spacing: Theme.s1
                IconButton {
                    visible: !win.sidebarOpen
                    label: qsTr("Open sidebar")
                    iconName: "panel-left"
                    onClicked: win.sidebarOpen = true
                }
                IconButton {
                    visible: !win.sidebarOpen
                    label: qsTr("New chat")
                    iconName: "square-pen"
                    onClicked: win.newChat()
                }
                IconButton {
                    visible: win.page !== "chat"
                    label: qsTr("Back to chat")
                    iconName: "arrow-left"
                    onClicked: win.go("chat")
                }
                Item {
                    Layout.fillWidth: true
                }
            }

            AttentionArea {
                id: attentionArea
                Layout.fillWidth: true
                Layout.leftMargin: Theme.s5
                Layout.rightMargin: Theme.s5
                Layout.topMargin: Theme.s3
                visible: win.blocking.length === 0 && Daemon.connected && hasContent
                onOpenPage: p => win.go(p)
            }

            FocusScope {
                Layout.fillWidth: true
                Layout.fillHeight: true
                focus: true

                ChatPage {
                    id: chat
                    anchors.fill: parent
                    visible: win.page === "chat" && win.blocking.length === 0
                    focus: win.page === "chat"
                    onChatOpened: id => sidebar.current = id
                    onToast: text => win.toast(text)
                }

                Loader {
                    id: settingsLoader
                    anchors.fill: parent
                    active: win.page === "settings" && win.blocking.length === 0
                    visible: active
                    focus: active
                    source: Qt.resolvedUrl("SettingsPage.qml")
                    onLoaded: {
                        item.title = qsTr("Settings")
                        const section = item as SectionPage
                        if (section && win.tab.length > 0)
                            section.show(win.tab)
                        toFocus.restart()
                        if (!Motion.reduced) {
                            item.opacity = 0
                            pageIn.target = item
                            pageIn.restart()
                        }
                    }
                }

                Loader {
                    id: aboutLoader
                    anchors.fill: parent
                    active: win.page === "about" && win.blocking.length === 0
                    visible: active
                    focus: active
                    source: Qt.resolvedUrl("AboutPage.qml")
                    onLoaded: {
                        item.title = qsTr("About")
                        if (!Motion.reduced) {
                            item.opacity = 0
                            pageIn.target = item
                            pageIn.restart()
                        }
                    }
                }

                Connections {
                    target: settingsLoader.item
                    ignoreUnknownSignals: true
                    function onNavigate(target) {
                        win.go(target)
                    }
                    function onToast(text) {
                        win.toast(text)
                    }
                    function onSearch(target, key) {
                        win.jump(target, key)
                    }
                }
                Connections {
                    target: aboutLoader.item
                    ignoreUnknownSignals: true
                    function onNavigate(target) {
                        win.go(target)
                    }
                    function onToast(text) {
                        win.toast(text)
                    }
                }

                NumberAnimation {
                    id: pageIn
                    property: "opacity"
                    to: 1
                    duration: Motion.overlay
                    easing.type: Easing.OutCubic
                }

                BlockingPanel {
                    anchors.fill: parent
                    visible: win.blocking.length > 0
                    reason: win.blocking
                }
            }
        }
    }

    PromptHost {
        onToast: text => win.toast(text)
    }

    ToastHost {
        id: toasts
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.margins: Theme.s5
        z: 100
    }
}
