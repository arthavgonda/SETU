import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

Page {
    id: page
    property var items: []
    property var catalog: []
    property bool loading: true
    property bool catalogLoading: false
    property string error
    property string catalogError
    property string tab: "installed"
    property var updates: ({})

    readonly property bool developer: Settings.values["plugins.developerMode"] === true
    readonly property var visibleItems: items.filter(p => developer || p.signature !== "unsigned")
    readonly property var invalid: items.filter(p => p.signature === "invalid")
    readonly property var signatureText: ({ verified: qsTr("Verified"), unsigned: qsTr("Unsigned"), invalid: qsTr("Invalid signature") })

    function load() {
        Daemon.call("plugins.list", {}, (ok, r, e) => {
            page.loading = false
            page.error = ok ? "" : e.message
            if (ok)
                page.items = r.items
        })
    }
    function loadCatalog() {
        catalogLoading = true
        Daemon.call("plugins.catalog", {}, (ok, r, e) => {
            page.catalogLoading = false
            page.catalogError = ok ? "" : e.message
            if (ok)
                page.catalog = r.items
        })
    }
    function remove(p) {
        Daemon.call("plugins.remove", { id: p.id }, (ok, r, e) => {
            if (!ok)
                page.toast(e.message)
        })
    }

    Component.onCompleted: if (Daemon.connected) load()
    Connections {
        target: Daemon
        function onReconnected() {
            page.load()
            if (page.tab === "browse")
                page.loadCatalog()
        }
    }
    Subscription {
        topics: ["plugins.state"]
        onReceived: (topic, d) => {
            page.load()
            if (d.status === "failed" && d.signature === "invalid")
                Announcer.announce(page, qsTr("A plugin failed signature verification and was disabled."), true)
        }
    }

    Banner {
        Layout.fillWidth: true
        visible: page.developer
        level: "warning"
        announce: false
        text: qsTr("Developer mode is on. Unsigned plugins can be installed and run with the permissions they ask for.")
        actionText: qsTr("Turn off")
        onAction: Settings.set("plugins.developerMode", false)
    }
    Repeater {
        model: page.invalid
        Banner {
            required property var modelData
            Layout.fillWidth: true
            level: "danger"
            text: qsTr("%1 failed signature verification and was disabled.").arg(modelData.name)
            actionText: qsTr("Remove")
            secondaryText: qsTr("Details")
            onAction: page.remove(modelData)
            onSecondary: page.toast(qsTr("%1 by %2, version %3. Its files do not match the publisher's signature, so it may have been changed after it was published.").arg(modelData.name).arg(modelData.publisher).arg(modelData.version))
        }
    }

    Tabs {
        Layout.fillWidth: true
        current: page.tab
        tabs: [{ id: "installed", title: qsTr("Installed") }, { id: "browse", title: qsTr("Browse") }]
        onActivated: id => {
            page.tab = id
            if (id === "browse" && page.catalog.length === 0)
                page.loadCatalog()
        }
    }

    ColumnLayout {
        Layout.fillWidth: true
        visible: page.tab === "installed"
        spacing: Theme.s3
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
        EmptyState {
            Layout.fillWidth: true
            visible: !page.loading && page.error.length === 0 && page.visibleItems.length === 0
            text: qsTr("No plugins installed. Browse the catalog to add support for more apps.")
            actionText: qsTr("Browse")
            onAction: {
                page.tab = "browse"
                page.loadCatalog()
            }
        }
        Repeater {
            model: page.visibleItems
            Rectangle {
                id: card
                required property var modelData
                readonly property var p: modelData
                Layout.fillWidth: true
                implicitHeight: pcol.implicitHeight + Theme.s4 * 2
                radius: Theme.radiusPanel
                color: Theme.bgRaised
                border.width: Theme.border
                border.color: p.signature === "invalid" ? Theme.danger : Theme.borderSubtle
                Accessible.role: Accessible.Grouping
                Accessible.name: p.name
                ColumnLayout {
                    id: pcol
                    anchors.fill: parent
                    anchors.margins: Theme.s4
                    spacing: Theme.s2
                    RowLayout {
                        Layout.fillWidth: true
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 0
                            Label {
                                Layout.fillWidth: true
                                text: card.p.name
                                role: "heading"
                                Accessible.role: Accessible.Heading
                            }
                            Label {
                                Layout.fillWidth: true
                                text: qsTr("%1  ·  version %2").arg(card.p.publisher).arg(card.p.version)
                                role: "small"
                                color: Theme.textSecondary
                            }
                        }
                        Switch {
                            text: card.p.enabled ? qsTr("On") : qsTr("Off")
                            Accessible.name: qsTr("Enable %1").arg(card.p.name)
                            checked: card.p.enabled
                            enabled: card.p.signature !== "invalid"
                            onToggled: Daemon.call("plugins.setEnabled", { id: card.p.id, enabled: checked }, (ok, r, e) => {
                                if (!ok)
                                    page.toast(e.message)
                                page.load()
                            })
                        }
                    }
                    StatusDot {
                        level: card.p.signature === "verified" ? "success" : card.p.signature === "invalid" ? "danger" : "warning"
                        showIcon: true
                        text: page.signatureText[card.p.signature] || card.p.signature
                    }
                    Label {
                        Layout.fillWidth: true
                        visible: card.p.can.length > 0
                        text: qsTr("Can: %1").arg(card.p.can.join(", "))
                    }
                    Label {
                        Layout.fillWidth: true
                        visible: card.p.access.length > 0
                        text: qsTr("Can access: %1").arg(card.p.access.join(", "))
                        color: Theme.textSecondary
                    }
                    RowLayout {
                        spacing: Theme.s2
                        Button {
                            text: qsTr("Check for update")
                            onClicked: Daemon.call("plugins.checkUpdate", { id: card.p.id }, (ok, r, e) => page.toast(!ok ? e.message : r.available ? qsTr("Version %1 is available.").arg(r.version) : qsTr("%1 is up to date.").arg(card.p.name)))
                        }
                        Button {
                            text: qsTr("Remove")
                            variant: "danger"
                            Accessible.name: qsTr("Remove %1").arg(card.p.name)
                            onClicked: page.remove(card.p)
                        }
                    }
                }
            }
        }
    }

    ColumnLayout {
        Layout.fillWidth: true
        visible: page.tab === "browse"
        spacing: Theme.s3
        LoadingState {
            Layout.fillWidth: true
            visible: page.catalogLoading
            what: qsTr("Plugin catalog")
        }
        ErrorState {
            Layout.fillWidth: true
            visible: !page.catalogLoading && page.catalogError.length > 0
            text: page.catalogError
            onAction: page.loadCatalog()
        }
        EmptyState {
            Layout.fillWidth: true
            visible: !page.catalogLoading && page.catalogError.length === 0 && page.catalog.length === 0
            text: qsTr("The signed catalog in this version has no plugins yet.")
        }
        Repeater {
            model: page.catalog
            ListRow {
                required property var modelData
                Layout.fillWidth: true
                interactive: false
                title: modelData.name + "  ·  " + modelData.publisher
                secondary: modelData.description + "  ·  " + Platform.formatBytes(modelData.sizeBytes)
                iconName: "puzzle"
                Button {
                    text: modelData.installed ? qsTr("Installed") : qsTr("Install")
                    enabled: !modelData.installed
                    Accessible.name: qsTr("Install %1").arg(modelData.name)
                    onClicked: Daemon.call("plugins.install", { id: modelData.id }, (ok, r, e) => {
                        if (!ok)
                            page.toast(e.message)
                    })
                }
            }
        }
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("Developer")
        SettingSwitch {
            objectName: "setting:plugins.developerMode"
            key: "plugins.developerMode"
            text: qsTr("Developer mode")
            description: qsTr("Shows and allows unsigned plugins. Only turn this on for plugins you are writing or trust.")
        }
    }
}
