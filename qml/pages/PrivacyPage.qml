import QtQuick
import QtQuick.Window
import QtQuick.Layouts
import QtQuick.Dialogs
import Setu.Core
import Setu.Theme
import Ledger

Page {
    id: page
    property var summary: null
    property bool loading: true
    property string error
    property string exportStore: ""
    property string viewStore: ""
    property var viewItems: []
    property string viewCursor: ""
    property bool viewLoading: false
    property var face: ({})

    function loadFace() {
        Daemon.call("face.state", {}, (ok, r) => {
            if (ok)
                page.face = r
        })
    }
    function run(text) {
        Daemon.call("intent.run", { text: text }, (ok, r, e) => {
            if (!ok && e)
                page.toast(e.message)
            faceTimer.restart()
        })
    }
    Timer {
        id: faceTimer
        interval: 1500
        onTriggered: page.loadFace()
    }
    function load() {
        page.loadFace()
        Daemon.call("data.summary", {}, (ok, r, e) => {
            page.loading = false
            page.error = ok ? "" : e.message
            if (ok)
                page.summary = r
        })
    }
    function view(store, more) {
        viewLoading = true
        if (!more) {
            viewStore = store.id
            viewItems = []
            viewCursor = ""
            viewDialog.title = store.name
            viewDialog.open()
        }
        let p = { store: viewStore, limit: 100 }
        if (more && viewCursor.length > 0)
            p.cursor = viewCursor
        Daemon.call("data.query", p, (ok, r, e) => {
            page.viewLoading = false
            if (!ok) {
                page.toast(e.message)
                return
            }
            page.viewItems = page.viewItems.concat(r.items)
            page.viewCursor = r.nextCursor || ""
        })
    }
    function exportTo(store) {
        exportStore = store
        saveDialog.selectedFile = "file://" + Platform.homePath + "/setu-" + (store.length > 0 ? store : "data") + ".json"
        saveDialog.open()
    }

    Component.onCompleted: if (Daemon.connected) load()
    Connections {
        target: Daemon
        function onReconnected() { page.load() }
    }
    Subscription {
        topics: ["history.added"]
        onReceived: () => refresh.restart()
    }
    Timer {
        id: refresh
        interval: 1000
        onTriggered: page.load()
    }

    FileDialog {
        id: saveDialog
        title: qsTr("Export data")
        fileMode: FileDialog.SaveFile
        nameFilters: [qsTr("JSON files (*.json)")]
        defaultSuffix: "json"
        onAccepted: {
            let p = { path: Platform.localFile(selectedFile) }
            if (page.exportStore.length > 0)
                p.store = page.exportStore
            Daemon.call("data.export", p, (ok, r, e) => page.toast(ok ? qsTr("Saved to %1").arg(r.path) : e.message))
        }
    }
    ConfirmDialog {
        id: confirm
    }
    Dialog {
        id: viewDialog
        preferredWidth: Theme.px(720)
        RowList {
            id: viewList
            Layout.fillWidth: true
            Layout.preferredHeight: Theme.px(360)
            fitContent: false
            label: viewDialog.title
            model: page.viewItems
            loading: page.viewLoading && page.viewItems.length === 0
            emptyText: qsTr("Nothing is stored here.")
            delegate: ListRow {
                required property var modelData
                width: ListView.view.width
                interactive: true
                mono: true
                title: Object.keys(modelData).filter(k => k !== "id").map(k => k + ": " + (typeof modelData[k] === "object" ? JSON.stringify(modelData[k]) : modelData[k])).join("  ·  ")
            }
        }
        Connections {
            target: viewList.view
            function onAtYEndChanged() {
                if (viewList.view.atYEnd && page.viewCursor.length > 0 && !page.viewLoading)
                    page.view(null, true)
            }
        }
        buttons: [
            Item { Layout.fillWidth: true },
            Button {
                text: qsTr("Close")
                onClicked: viewDialog.close()
            }
        ]
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("What is stored")
        description: qsTr("Everything stays on this computer.")
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
            model: page.summary ? page.summary.stores : []
            ListRow {
                required property var modelData
                Layout.fillWidth: true
                interactive: false
                title: modelData.name
                secondary: qsTr("%n items", "", modelData.items) + "  ·  " + Platform.formatBytes(modelData.bytes)
                iconName: "database"
                Button {
                    variant: "quiet"
                    text: qsTr("View")
                    Accessible.name: qsTr("View %1").arg(modelData.name)
                    onClicked: page.view(modelData, false)
                }
                Button {
                    variant: "quiet"
                    text: qsTr("Export")
                    Accessible.name: qsTr("Export %1").arg(modelData.name)
                    onClicked: page.exportTo(modelData.id)
                }
                Button {
                    variant: "danger"
                    text: qsTr("Delete")
                    Accessible.name: qsTr("Delete %1").arg(modelData.name)
                    enabled: modelData.items > 0
                    onClicked: confirm.ask({
                        title: qsTr("Delete %1").arg(modelData.name.toLowerCase()),
                        facts: [qsTr("%n items", "", modelData.items) + "  " + Platform.formatBytes(modelData.bytes)],
                        consequence: modelData.id === "apps" ? qsTr("Names you added and how often you opened each app are forgotten. The apps themselves stay installed.") : qsTr("This can't be undone."),
                        actionLabel: qsTr("Delete"),
                        danger: true,
                        accept: () => Daemon.call("data.wipe", { store: modelData.id }, (ok, r, e) => {
                            page.toast(ok ? qsTr("Deleted.") : e.message)
                            page.load()
                        })
                    })
                }
            }
        }
        Label {
            Layout.fillWidth: true
            text: qsTr("Audio: never stored. This can't be turned on in this version.")
            Accessible.role: Accessible.StaticText
        }
        Button {
            text: qsTr("Export everything")
            iconName: "file-down"
            onClicked: page.exportTo("")
        }
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("Face unlock and passwords")
        Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            color: Theme.textSecondary
            text: page.face.enrolled ? (page.face.system ? qsTr("Your face is set up. Saved passwords are shown or typed only after Setu sees it's you.") : qsTr("Your face is set up. Add your system password so Setu can type it for you.")) : qsTr("Not set up. Setu learns your face from the camera and keeps only face measurements, never pictures.")
        }
        RowLayout {
            spacing: Theme.s2
            Button {
                variant: "primary"
                text: page.face.enrolled ? qsTr("Set up again") : qsTr("Set up face unlock")
                onClicked: page.run("set up face unlock")
            }
            Button {
                visible: page.face.enrolled === true && page.face.system !== true
                text: qsTr("Add system password")
                onClicked: page.run("save my system password")
            }
            Button {
                visible: page.face.enrolled === true
                variant: "quiet"
                text: qsTr("Forget my face")
                onClicked: page.run("forget my face")
            }
        }
        SettingSwitch {
            objectName: "setting:face.unlock"
            key: "face.unlock"
            visible: Platform.os === "linux"
            text: qsTr("Unlock the lock screen when it sees me")
            description: qsTr("When you wake the lock screen, Setu checks your face and opens it. A photo is refused, but a video of you on a screen may not be.")
        }
        Switch {
            objectName: "face.signin"
            Layout.fillWidth: true
            visible: Platform.os === "linux" && page.face.enrolled === true
            text: qsTr("Sign in with my face")
            description: page.face.helper === true ? qsTr("At the sign-in screen, pick your name and look at the camera. Your system password is kept sealed on this computer for that screen.") : qsTr("Needs admin control first, because the sign-in screen runs before you are signed in.")
            checked: page.face.signin === true
            onToggled: {
                page.run(checked ? "use my face to sign in" : "turn off face sign-in")
                checked = Qt.binding(() => page.face.signin === true)
            }
        }
        Switch {
            objectName: "face.admin"
            Layout.fillWidth: true
            visible: Platform.os === "linux" && page.face.enrolled === true
            text: qsTr("Approve admin prompts with my face and a nod")
            description: qsTr("sudo and admin prompts take your face and a nod instead of the password. Three refusals and they want the password again.")
            checked: page.face.admin === true
            onToggled: {
                page.run(checked ? "use my face for admin prompts" : "stop using my face for admin prompts")
                checked = Qt.binding(() => page.face.admin === true)
            }
        }
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("Learning and history")
        SettingSwitch {
            objectName: "setting:privacy.learn"
            key: "privacy.learn"
            text: qsTr("Learn from my commands")
            description: qsTr("Remembers your corrections, favourite apps and how you say things, on this computer only.")
        }
        SettingSwitch {
            objectName: "setting:history.keep"
            key: "history.keep"
            text: qsTr("Keep history")
        }
        SettingSelect {
            objectName: "setting:history.retention"
            key: "history.retention"
            enabled: Settings.values["history.keep"] !== false
            label: qsTr("Keep history for")
            options: [
                { value: "30d", text: qsTr("30 days") },
                { value: "1y", text: qsTr("1 year") },
                { value: "forever", text: qsTr("Until I delete it") }
            ]
        }
        SettingSwitch {
            objectName: "setting:privacy.crashReports"
            key: "privacy.crashReports"
            text: qsTr("Share anonymous crash reports")
            description: qsTr("Sends the error and versions only. Never what you said, your files or your history.")
        }
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("Data folder")
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s2
            Label {
                Layout.fillWidth: true
                text: page.summary ? page.summary.dataDir : (Daemon.paths.data || "")
                role: "mono"
                wrapMode: Text.WrapAnywhere
            }
            Button {
                text: qsTr("Open folder")
                iconName: "folder-open"
                onClicked: Platform.openPath(page.summary ? page.summary.dataDir : Daemon.paths.data)
            }
        }
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("Reset")
        Button {
            objectName: "setting:data.wipe"
            text: qsTr("Delete everything and reset")
            variant: "danger"
            onClicked: confirm.ask({
                title: qsTr("Delete everything and reset"),
                facts: (page.summary ? page.summary.stores.map(s => s.name + "  " + qsTr("%n items", "", s.items)) : []).concat([qsTr("All settings")]),
                consequence: qsTr("%1 forgets everything and starts setup again. Downloaded models are kept.").arg(Brand.productName),
                actionLabel: qsTr("Delete everything"),
                danger: true,
                word: "DELETE",
                accept: () => Daemon.call("data.wipe", { confirm: "DELETE" }, (ok, r, e) => {
                    if (!ok) {
                        page.toast(e.message)
                        return
                    }
                    for (const slot of ["asr", "llm", "tts"])
                        SecretStore.remove("api." + slot)
                    page.navigate("wizard")
                })
            })
        }
    }
}
