import QtQuick
import QtQuick.Window
import QtQuick.Layouts
import QtQuick.Dialogs
import Setu.Core
import Setu.Theme
import Ledger

Page {
    id: page
    readonly property var hostWindow: Window.window
    property var tiers: []
    property var automation: []
    property bool loading: true
    property string error

    readonly property var tierInfo: ({
        basic: { name: qsTr("Basic"), key: "", text: qsTr("Hearing you, opening apps and changing volume or brightness.") },
        control: { name: qsTr("Control"), key: "permissions.control", text: qsTr("Reading and clicking controls in other apps, and reading the screen.") },
        admin: { name: qsTr("Admin"), key: "permissions.admin", text: qsTr("Installing apps and changing system settings. Always asked for each action.") }
    })
    readonly property var folders: Settings.values["files.folders"] || []

    function load() {
        Daemon.call("permissions.status", {}, (ok, r, e) => {
            page.loading = false
            page.error = ok ? "" : e.message
            if (ok) {
                page.tiers = r.tiers
                page.automation = r.automation
            }
        })
    }
    function request(item) {
        Daemon.call("permissions.request", { item: item }, (ok, r, e) => {
            if (!ok)
                page.toast(e.message)
            else if (r.opened)
                page.toast(qsTr("System settings are open. The status here updates when you come back."))
            page.load()
        })
    }
    function setFolders(list) {
        Settings.set("files.folders", list)
    }

    Component.onCompleted: if (Daemon.connected) load()
    Connections {
        target: Daemon
        function onReconnected() { page.load() }
    }
    Connections {
        target: page.hostWindow
        function onActiveChanged() {
            if (page.hostWindow && page.hostWindow.active)
                page.load()
        }
    }
    Subscription {
        topics: ["permissions.state"]
        onReceived: () => page.load()
    }
    Connections {
        target: Settings
        function onValuesChanged() { reloadSoon.restart() }
    }
    Timer {
        id: reloadSoon
        interval: 300
        onTriggered: page.load()
    }

    FolderDialog {
        id: folderDialog
        title: qsTr("Choose a folder %1 may use").arg(Brand.assistantName)
        currentFolder: "file://" + Platform.homePath
        onAccepted: {
            const p = Platform.localFile(selectedFolder)
            if (p.length > 0 && page.folders.indexOf(p) < 0)
                page.setFolders(page.folders.concat([p]))
        }
    }
    ConfirmDialog {
        id: confirm
    }

    LoadingState {
        Layout.fillWidth: true
        visible: page.loading
        what: page.title
    }
    ErrorState {
        Layout.fillWidth: true
        visible: !page.loading && page.error.length > 0
        text: page.error
        onAction: page.load()
    }

    Repeater {
        model: page.tiers
        ColumnLayout {
            id: tierBlock
            required property var modelData
            readonly property var info: page.tierInfo[modelData.id] || { name: modelData.id, key: "", text: "" }
            Layout.fillWidth: true
            spacing: Theme.s2
            Item {
                Layout.fillWidth: true
                implicitHeight: disclosure.implicitHeight
                Disclosure {
                    id: disclosure
                    width: parent.width
                    title: tierBlock.info.name
                    expanded: tierBlock.modelData.id === "basic" || tierBlock.modelData.enabled
                    Label {
                        Layout.fillWidth: true
                        text: tierBlock.info.text
                        color: Theme.textSecondary
                    }
                    Repeater {
                        model: tierBlock.modelData.items
                        PermissionCard {
                            required property var modelData
                            Layout.fillWidth: true
                            name: modelData.name
                            allows: modelData.allows
                            never: modelData.never
                            status: modelData.status
                            fix: modelData.fix || ""
                            required: modelData.required
                            onRequested: fix => {
                                if (modelData.id === "folders" && page.folders.length === 0)
                                    folderDialog.open()
                                page.request(modelData.id)
                            }
                        }
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        visible: tierBlock.modelData.id === "files"
                        spacing: Theme.s2
                        Label {
                            text: qsTr("Folders")
                            role: "label"
                        }
                        Repeater {
                            model: page.folders
                            ListRow {
                                required property string modelData
                                required property int index
                                Layout.fillWidth: true
                                interactive: false
                                title: modelData
                                mono: true
                                iconName: "folder"
                                IconButton {
                                    label: qsTr("Remove %1").arg(modelData)
                                    iconName: "x"
                                    onClicked: {
                                        let l = page.folders.slice()
                                        l.splice(index, 1)
                                        page.setFolders(l)
                                    }
                                }
                            }
                        }
                        Label {
                            visible: page.folders.length === 0
                            text: qsTr("No folders yet.")
                            color: Theme.textSecondary
                        }
                        Button {
                            text: qsTr("Add a folder")
                            iconName: "folder-open"
                            onClicked: folderDialog.open()
                        }
                    }
                }
                Switch {
                    anchors.right: parent.right
                    anchors.top: parent.top
                    objectName: tierBlock.info.key.length > 0 ? "setting:" + tierBlock.info.key : ""
                    visible: tierBlock.info.key.length > 0
                    text: tierBlock.modelData.enabled ? qsTr("On") : qsTr("Off")
                    Accessible.name: qsTr("Allow %1").arg(tierBlock.info.name)
                    checked: tierBlock.modelData.enabled
                    onToggled: {
                        if (tierBlock.info.key === "permissions.control" && checked)
                            page.request("accessibility")
                        else
                            Settings.set(tierBlock.info.key, checked)
                    }
                }
            }
            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 1
                color: Theme.borderSubtle
            }
        }
    }

    Section {
        Layout.fillWidth: true
        visible: Platform.os === "macos"
        title: qsTr("Apps I can control")
        description: qsTr("Apps that have approved %1 through macOS Automation.").arg(Brand.productName)
        Repeater {
            model: page.automation
            Switch {
                required property var modelData
                Layout.fillWidth: true
                text: qsTr("Allow %1 to control %2").arg(Brand.productName).arg(modelData.app)
                checked: modelData.allowed
                onToggled: page.request("automation")
            }
        }
        Label {
            visible: page.automation.length === 0
            text: qsTr("No apps yet. macOS asks the first time %1 controls an app.").arg(Brand.productName)
            color: Theme.textSecondary
        }
    }

    Button {
        objectName: "setting:permissions.reset"
        text: qsTr("Reset all permissions")
        variant: "danger"
        onClicked: confirm.ask({
            title: qsTr("Reset all permissions"),
            facts: [qsTr("Files, Control and screen reading are turned off"), qsTr("%n chosen folders are removed", "", page.folders.length)],
            consequence: qsTr("%1 asks again before using any of them. Permissions you granted in the system settings stay until you remove them there.").arg(Brand.productName),
            actionLabel: qsTr("Reset"),
            danger: true,
            accept: () => Daemon.call("permissions.reset", {}, (ok, r, e) => {
                page.toast(ok ? qsTr("Permissions reset.") : e.message)
                page.load()
            })
        })
    }
}
