import QtQuick
import QtQuick.Window
import QtQuick.Layouts
import QtQuick.Dialogs
import Setu.Core
import Setu.Theme
import Ledger

Page {
    id: page
    showSaved: false
    property var items: []
    property var missing: []
    property var defaults: ({})
    property var candidates: ({})
    property bool scanning: false
    property bool loading: true
    property string error
    property bool showHidden: false
    property var editing: null
    property var aliasDraft: []

    readonly property var roles: [
        { id: "music", name: qsTr("Music") },
        { id: "browser", name: qsTr("Browser") },
        { id: "editor", name: qsTr("Editor") },
        { id: "mail", name: qsTr("Mail") },
        { id: "terminal", name: qsTr("Terminal") },
        { id: "files", name: qsTr("Files") }
    ]
    readonly property var sourceText: ({ start_menu: qsTr("Start menu"), applications: qsTr("Applications"), desktop: qsTr(".desktop"), store: qsTr("Store"), flatpak: qsTr("Flatpak"), snap: qsTr("Snap"), manual: qsTr("Added by you") })
    readonly property var shown: {
        const q = filter.text.trim().toLowerCase()
        return items.filter(a => (showHidden || !a.hidden) && (q.length === 0 || a.name.toLowerCase().includes(q) || a.aliases.some(x => x.toLowerCase().includes(q))))
    }

    function load() {
        Daemon.call("apps.list", { includeHidden: true }, (ok, r, e) => {
            page.loading = false
            page.error = ok ? "" : e.message
            if (!ok)
                return
            page.items = r.items.sort((a, b) => a.name.localeCompare(b.name))
            page.missing = r.missing
            page.defaults = r.defaults
            page.candidates = r.candidates
            page.scanning = r.scanning
        })
    }
    function appName(id) {
        const a = items.find(x => x.id === id)
        return a ? a.name : id
    }
    function roleOptions(role) {
        let ids = (candidates[role] || []).slice()
        if (defaults[role] && ids.indexOf(defaults[role]) < 0)
            ids.unshift(defaults[role])
        return [{ value: "", text: qsTr("Ask me each time") }].concat(ids.map(id => ({ value: id, text: appName(id) })))
    }
    function setDefault(role, id) {
        Daemon.call("apps.setDefault", { role: role, id: id.length > 0 ? id : null }, (ok, r, e) => {
            if (ok)
                page.load()
            else
                page.toast(e.message)
        })
    }
    function rescan() {
        page.scanning = true
        Daemon.call("apps.rescan", {}, (ok, r, e) => {
            page.scanning = false
            page.toast(ok ? qsTr("Found %n apps.", "", r.count) : e.message)
            page.load()
        })
    }
    function editAliases(app) {
        editing = app
        aliasDraft = app.aliases.slice()
        aliasInput.text = ""
        aliasDialog.open()
    }

    Component.onCompleted: if (Daemon.connected) load()
    Connections {
        target: Daemon
        function onReconnected() { page.load() }
    }
    Subscription {
        topics: ["apps.changed"]
        onReceived: (topic, d) => {
            page.scanning = d.scanning
            if (!d.scanning)
                page.load()
        }
    }

    FileDialog {
        id: addDialog
        title: qsTr("Choose an app")
        fileMode: FileDialog.OpenFile
        currentFolder: Platform.os === "linux" ? "file:///usr/share/applications" : Platform.os === "macos" ? "file:///Applications" : "file:///C:/Program Files"
        nameFilters: Platform.os === "linux" ? [qsTr("App launchers (*.desktop)"), qsTr("All files (*)")] : Platform.os === "windows" ? [qsTr("Programs (*.exe *.lnk)")] : [qsTr("Apps (*.app)")]
        onAccepted: Daemon.call("apps.add", { path: Platform.localFile(selectedFile) }, (ok, r, e) => {
            page.toast(ok ? qsTr("Added %1.").arg(r.name) : e.message)
            if (ok)
                page.load()
        })
    }

    Dialog {
        id: aliasDialog
        title: page.editing ? qsTr("Names for %1").arg(page.editing.name) : ""
        initialFocus: aliasInput
        Label {
            Layout.fillWidth: true
            text: qsTr("Other names you can say for this app. %1 always understands its real name.").arg(Brand.assistantName)
            color: Theme.textSecondary
        }
        Flow {
            Layout.fillWidth: true
            spacing: Theme.s2
            visible: page.aliasDraft.length > 0
            Repeater {
                model: page.aliasDraft
                Chip {
                    required property string modelData
                    required property int index
                    text: modelData
                    removable: true
                    Accessible.description: qsTr("Press Delete to remove")
                    onRemoveRequested: {
                        let d = page.aliasDraft.slice()
                        d.splice(index, 1)
                        page.aliasDraft = d
                        aliasInput.forceActiveFocus()
                    }
                }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s2
            TextField {
                id: aliasInput
                Layout.fillWidth: true
                label: qsTr("Add a name")
                maximumLength: 40
                error: page.aliasDraft.some(a => a.toLowerCase() === text.trim().toLowerCase()) ? qsTr("That name is already in the list.") : ""
                onAccepted: addAlias.clicked()
            }
            Button {
                id: addAlias
                Layout.alignment: Qt.AlignBottom
                text: qsTr("Add")
                enabled: aliasInput.text.trim().length > 0 && aliasInput.error.length === 0 && page.aliasDraft.length < 10
                onClicked: {
                    page.aliasDraft = page.aliasDraft.concat([aliasInput.text.trim()])
                    aliasInput.text = ""
                }
            }
        }
        buttons: [
            Button {
                text: qsTr("Cancel")
                onClicked: aliasDialog.close()
            },
            Item { Layout.fillWidth: true },
            Button {
                variant: "primary"
                text: qsTr("Save")
                onClicked: Daemon.call("apps.setAlias", { id: page.editing.id, aliases: page.aliasDraft }, (ok, r, e) => {
                    if (ok) {
                        aliasDialog.close()
                        page.load()
                    } else {
                        page.toast(e.message)
                    }
                })
            }
        ]
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("Defaults")
        description: qsTr("Used when you say a kind of app, such as \"play music\" or \"open the browser\".")
        GridLayout {
            Layout.fillWidth: true
            columns: width > Theme.px(560) ? 2 : 1
            columnSpacing: Theme.s4
            rowSpacing: Theme.s3
            Repeater {
                model: page.roles
                Select {
                    required property var modelData
                    Layout.fillWidth: true
                    objectName: "setting:apps.defaults." + modelData.id
                    label: modelData.name
                    options: page.roleOptions(modelData.id)
                    value: page.defaults[modelData.id] || ""
                    onActivated: v => page.setDefault(modelData.id, v)
                }
            }
        }
    }

    Section {
        Layout.fillWidth: true
        visible: page.missing.length > 0
        title: qsTr("Not found")
        description: qsTr("Apps you asked for that aren't on this computer.")
        Repeater {
            model: page.missing
            ListRow {
                required property var modelData
                Layout.fillWidth: true
                interactive: false
                title: modelData.name
                secondary: qsTr("Asked %n times, most recently %1", "", modelData.count).arg(Platform.formatDateTime(modelData.lastAsked))
                iconName: "search"
                Button {
                    text: qsTr("Install")
                    Accessible.name: qsTr("Install %1").arg(modelData.name)
                    onClicked: Daemon.call("apps.install", { name: modelData.name }, (ok, r, e) => {
                        if (!ok)
                            page.toast(e.message)
                    })
                }
                Button {
                    variant: "quiet"
                    text: qsTr("Dismiss")
                    Accessible.name: qsTr("Dismiss %1").arg(modelData.name)
                    onClicked: Daemon.call("apps.dismissMissing", { name: modelData.name }, () => page.load())
                }
            }
        }
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("Apps on this computer")
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s2
            SearchField {
                id: filter
                Layout.fillWidth: true
                label: qsTr("Filter apps")
            }
            Button {
                Layout.alignment: Qt.AlignBottom
                text: page.scanning ? qsTr("Scanning") : qsTr("Rescan")
                iconName: "refresh-cw"
                busy: page.scanning
                enabled: !page.scanning
                onClicked: page.rescan()
            }
            Button {
                Layout.alignment: Qt.AlignBottom
                text: qsTr("Add an app manually")
                iconName: "plus"
                onClicked: addDialog.open()
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Label {
                Layout.fillWidth: true
                role: "small"
                color: Theme.textSecondary
                text: qsTr("%n apps", "", page.shown.length) + (page.items.some(a => a.hidden) ? "  ·  " + qsTr("%n hidden", "", page.items.filter(a => a.hidden).length) : "")
            }
            Checkbox {
                text: qsTr("Show hidden apps")
                checked: page.showHidden
                onToggled: page.showHidden = checked
            }
        }
        RowList {
            Layout.fillWidth: true
            label: qsTr("Apps")
            maxVisible: 12
            model: page.shown
            loading: page.loading
            error: page.error
            emptyText: filter.text.length > 0 ? qsTr("No apps match \"%1\".").arg(filter.text) : qsTr("No apps found. Rescan, or add one by choosing its file.")
            emptyAction: filter.text.length > 0 ? "" : qsTr("Rescan")
            onEmptyActivated: page.rescan()
            onRetry: page.load()
            delegate: ListRow {
                id: row
                required property var modelData
                width: ListView.view.width
                title: modelData.name + (modelData.hidden ? "  " + qsTr("(hidden)") : "")
                secondary: [page.sourceText[modelData.source] || modelData.source,
                            modelData.aliases.length > 0 ? qsTr("Also: %1").arg(modelData.aliases.join(", ")) : "",
                            qsTr("Opened %n times", "", modelData.opened),
                            page.roles.filter(r => page.defaults[r.id] === modelData.id).map(r => qsTr("Default %1").arg(r.name.toLowerCase())).join(", ")].filter(s => s.length > 0).join("  ·  ")
                onClicked: page.editAliases(modelData)
                Button {
                    variant: "quiet"
                    text: qsTr("Names")
                    Accessible.name: qsTr("Edit names for %1").arg(row.modelData.name)
                    focusPolicy: Qt.TabFocus
                    onClicked: page.editAliases(row.modelData)
                }
                IconButton {
                    label: qsTr("More for %1").arg(row.modelData.name)
                    iconName: "ellipsis"
                    onClicked: more.open()
                    Menu {
                        id: more
                        y: parent.height
                        Instantiator {
                            model: page.roles
                            delegate: MenuItem {
                                required property var modelData
                                text: qsTr("Use as default %1").arg(modelData.name.toLowerCase())
                                onTriggered: page.setDefault(modelData.id, row.modelData.id)
                            }
                            onObjectAdded: (index, object) => more.insertItem(index, object)
                            onObjectRemoved: (index, object) => more.removeItem(object)
                        }
                        MenuItem {
                            text: row.modelData.hidden ? qsTr("Show again") : qsTr("Hide")
                            onTriggered: Daemon.call("apps.hide", { id: row.modelData.id, hidden: !row.modelData.hidden }, (ok, r, e) => {
                                if (ok)
                                    page.load()
                                else
                                    page.toast(e.message)
                            })
                        }
                    }
                }
            }
        }
    }
}
