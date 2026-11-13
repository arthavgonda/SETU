import QtQuick
import QtQuick.Window
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

Page {
    id: page
    property bool checking: false

    readonly property var shortcuts: [
        { keys: [Platform.shortcut("Ctrl"), "F"], what: qsTr("Find a setting") },
        { keys: [Platform.shortcut("Ctrl"), ","], what: qsTr("Open settings") },
        { keys: [Platform.shortcut("Ctrl"), "Shift", "O"], what: qsTr("Start a new chat") },
        { keys: [Platform.shortcut("Ctrl"), "K"], what: qsTr("Search chats") },
        { keys: ["F6"], what: qsTr("Move between the chat list and the chat") },
        { keys: ["Esc"], what: qsTr("Stop, while the listening overlay is shown") },
        { keys: ["1", "–", "4"], what: qsTr("Choose a numbered answer, while a question is shown") },
        { keys: ["Enter"], what: qsTr("Confirm, when the confirmation method allows keys") },
        { keys: ["Enter"], what: qsTr("Send a message. Shift and Enter starts a new line") }
    ]
    readonly property var update: Daemon.update
    property var browsers: []
    property string browserState: ""
    property string browserMessage: ""
    readonly property var browser: browsers.find(b => b.chosen) || null

    function loadBrowsers() {
        Daemon.call("browsers.list", {}, (ok, r) => {
            if (ok)
                page.browsers = r.items
        })
    }

    Component.onCompleted: loadBrowsers()

    Subscription {
        topics: ["browsers.state"]
        onReceived: (topic, d) => {
            if (d.items)
                page.browsers = d.items
            if (d.setup) {
                page.browserState = d.setup
                page.browserMessage = d.message || ""
            }
        }
    }

    DiagnosticsDialog {
        id: diagnostics
        onToast: text => page.toast(text)
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("Language")
        Select {
            objectName: "setting:ui.language"
            Layout.fillWidth: true
            label: qsTr("Language")
            options: Translator.languages.map(l => ({ value: l.code, text: l.native === l.english ? l.native : l.native + " (" + l.english + ")" }))
            value: Settings.values["ui.language"] || Translator.language
            onActivated: v => {
                Settings.set("ui.language", v)
                Translator.apply(v)
            }
        }
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("Browser")
        description: qsTr("The browser %1 opens pages in and controls.").arg(Brand.productName)
        Select {
            objectName: "setting:web.browser"
            Layout.fillWidth: true
            visible: page.browsers.length > 0
            label: qsTr("Browser")
            options: (page.browser ? [] : [{ value: "", text: qsTr("Choose a browser") }]).concat(page.browsers.map(b => ({ value: b.id, text: b.name })))
            value: page.browser ? page.browser.id : ""
            onActivated: v => {
                if (!v)
                    return
                page.browserState = "working"
                Daemon.call("browsers.choose", { id: v }, (ok, r, e) => {
                    page.browserState = ok ? r.state : "failed"
                    page.browserMessage = ok ? "" : e.message
                })
            }
        }
        Label {
            Layout.fillWidth: true
            visible: page.browsers.length === 0
            text: qsTr("No browser found.")
            color: Theme.textSecondary
        }
        RowLayout {
            Layout.fillWidth: true
            visible: page.browser !== null
            spacing: Theme.s3
            StatusDot {
                level: page.browser && page.browser.connected ? "success" : page.browserState === "working" ? "info" : "warning"
                text: page.browser && page.browser.connected ? qsTr("Connected") : page.browserState === "working" ? qsTr("Setting up") : qsTr("Not connected yet")
                showIcon: true
            }
            Item {
                Layout.fillWidth: true
            }
            Button {
                visible: page.browser !== null && !page.browser.connected
                text: qsTr("Set up")
                busy: page.browserState === "working"
                onClicked: {
                    page.browserState = "working"
                    Daemon.call("browsers.setup", { id: page.browser.id }, (ok, r, e) => {
                        page.browserState = ok ? r.state : "failed"
                        page.browserMessage = ok ? "" : e.message
                    })
                }
            }
        }
        RowLayout {
            id: setuRow
            Layout.fillWidth: true
            visible: page.browser !== null && !!page.browser.setuProfile
            spacing: Theme.s3
            readonly property var sp: page.browser && page.browser.setuProfile ? page.browser.setuProfile : ({})
            Rectangle {
                Layout.preferredWidth: Theme.px(132)
                Layout.preferredHeight: Theme.px(64)
                radius: Theme.radiusPanel
                clip: true
                color: Theme.bgSunken
                Image {
                    anchors.fill: parent
                    source: setuRow.sp.background ? "qrc:/looks/thumbs/" + setuRow.sp.background + ".jpg" : ""
                    fillMode: Image.PreserveAspectCrop
                    asynchronous: true
                    Accessible.ignored: true
                }
                LookAvatar {
                    anchors.centerIn: parent
                    width: Theme.px(48)
                    height: width
                    avatar: setuRow.sp.avatar || ""
                    file: setuRow.sp.custom || ""
                }
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2
                Label {
                    Layout.fillWidth: true
                    text: setuRow.sp.name || qsTr("Setu profile")
                    role: "label"
                }
                Label {
                    Layout.fillWidth: true
                    text: setuRow.sp.isDefault ? qsTr("Your Setu profile, opened by default, with full control") : qsTr("Your Setu profile. The browser opens on your old profile.")
                    role: "small"
                    color: Theme.textSecondary
                    wrapMode: Text.WordWrap
                }
            }
            Button {
                text: qsTr("Change look")
                onClicked: Daemon.call("look.act", { op: "edit", browser: page.browser.name }, (ok, r, e) => {
                    if (!ok)
                        page.browserMessage = e.message
                })
            }
        }
        Label {
            Layout.fillWidth: true
            visible: page.browserMessage.length > 0 && page.browserState !== "working"
            text: page.browserMessage
            role: "small"
            color: Theme.textSecondary
        }
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("Start-up and tray")
        SettingSwitch {
            objectName: "setting:startup.atLogin"
            key: "startup.atLogin"
            text: qsTr("Start when I sign in")
        }
        Button {
            text: qsTr("Run setup again")
            iconName: "rotate-ccw"
            onClicked: Settings.patch({ "setup.completed": false, "setup.step": 1 })
        }
        SettingSwitch {
            objectName: "setting:general.switchInput"
            key: "general.switchInput"
            text: qsTr("Switch input")
        }
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("Keyboard shortcuts")
        Repeater {
            model: page.shortcuts
            RowLayout {
                objectName: index === 0 ? "setting:shortcuts" : ""
                required property var modelData
                required property int index
                Layout.fillWidth: true
                spacing: Theme.s3
                Accessible.role: Accessible.StaticText
                Accessible.name: modelData.keys.join(" ") + ": " + modelData.what
                KeyHint {
                    Layout.preferredWidth: Theme.px(150)
                    keys: modelData.keys
                    Accessible.ignored: true
                }
                Label {
                    Layout.fillWidth: true
                    text: modelData.what
                    Accessible.ignored: true
                }
            }
        }
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("Updates")
        Segmented {
            objectName: "setting:update.channel"
            Layout.fillWidth: true
            label: qsTr("Update channel")
            value: Settings.values["update.channel"] ?? "stable"
            options: [{ value: "stable", text: qsTr("Stable") }, { value: "beta", text: qsTr("Beta") }]
            onActivated: v => Settings.set("update.channel", v)
        }
        Label {
            Layout.fillWidth: true
            text: {
                const u = page.update
                if (!u || !u.current)
                    return ""
                if (u.failed)
                    return qsTr("The update to %1 failed. You are still on %2.").arg(u.failedVersion || "").arg(u.current)
                if (u.available)
                    return qsTr("Version %1 is available. You have %2.").arg(u.version).arg(u.current)
                if (!u.configured)
                    return qsTr("You have version %1. This build is updated through your package manager or the download page.").arg(u.current)
                return qsTr("You have the latest version, %1.").arg(u.current)
            }
            color: page.update && page.update.failed ? Theme.danger : Theme.textPrimary
        }
        RowLayout {
            spacing: Theme.s2
            Button {
                objectName: "setting:update.check"
                text: qsTr("Check for updates")
                iconName: "refresh-cw"
                busy: page.checking
                enabled: !page.checking
                onClicked: {
                    page.checking = true
                    Daemon.call("update.check", {}, (ok, r, e) => {
                        page.checking = false
                        if (!ok)
                            page.toast(e.message)
                    })
                }
            }
            Button {
                visible: page.update && page.update.available === true
                variant: "primary"
                text: qsTr("Install update")
                onClicked: Daemon.call("update.apply", {}, (ok, r, e) => {
                    if (!ok)
                        page.toast(e.message)
                })
            }
        }
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("Logs and diagnostics")
        RowLayout {
            spacing: Theme.s2
            Button {
                text: qsTr("Open logs folder")
                iconName: "folder-open"
                onClicked: Platform.openPath(Daemon.paths.logs || Launcher.logsFolder())
            }
            Button {
                objectName: "setting:diagnostics"
                text: qsTr("Export diagnostics")
                iconName: "file-text"
                onClicked: diagnostics.start()
            }
        }
    }
}
