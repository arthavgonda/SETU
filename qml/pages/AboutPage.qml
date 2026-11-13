import QtQuick
import QtQuick.Window
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

Page {
    id: page
    showSaved: false
    property var licenses: null
    property var shownText: null
    property bool deleteHistory: false
    property bool deleteModels: true
    property real modelBytes: 0
    readonly property var matches: {
        if (!licenses)
            return []
        const q = filter.text.trim().toLowerCase()
        return licenses.components.filter(c => q.length === 0 || c.name.toLowerCase().includes(q) || String(c.license).toLowerCase().includes(q) || String(c.kind).toLowerCase().includes(q))
    }

    function loadLicenses() {
        if (licenses)
            return
        const t = Platform.resourceText("licenses/third_party.json")
        try {
            licenses = JSON.parse(t)
        } catch (e) {
            licenses = { components: [], texts: {} }
        }
    }
    function showText(title, body) {
        shownText = { title: title, body: body }
        textDialog.title = title
        textDialog.open()
    }

    Dialog {
        id: textDialog
        preferredWidth: Theme.px(760)
        Scroll {
            Layout.fillWidth: true
            Layout.preferredHeight: Theme.px(420)
            gutter: 0
            TextEdit {
                width: parent.width
                readOnly: true
                selectByMouse: true
                textFormat: TextEdit.PlainText
                wrapMode: TextEdit.Wrap
                font.family: Type.mono
                font.pixelSize: Type.code.size
                color: Theme.textPrimary
                selectionColor: Theme.accent
                selectedTextColor: Theme.textOnAccent
                text: page.shownText ? page.shownText.body : ""
                activeFocusOnTab: true
                Accessible.role: Accessible.EditableText
                Accessible.name: textDialog.title
                Accessible.readOnly: true
            }
        }
        buttons: [
            Item { Layout.fillWidth: true },
            Button {
                text: qsTr("Close")
                onClicked: textDialog.close()
            }
        ]
    }

    Dialog {
        id: uninstallDialog
        title: qsTr("Uninstall %1").arg(Brand.productName)
        Label {
            Layout.fillWidth: true
            text: qsTr("%1 stops listening, stops starting at sign-in and is removed from this computer.").arg(Brand.productName)
        }
        Checkbox {
            Layout.fillWidth: true
            text: qsTr("Also delete history, learned preferences and corrections")
            checked: page.deleteHistory
            onToggled: page.deleteHistory = checked
        }
        Checkbox {
            Layout.fillWidth: true
            text: qsTr("Delete downloaded models, freeing %1").arg(Platform.formatBytes(page.modelBytes))
            checked: page.deleteModels
            onToggled: page.deleteModels = checked
        }
        Label {
            id: uninstallNote
            Layout.fillWidth: true
            visible: text.length > 0
            color: Theme.textSecondary
        }
        buttons: [
            Button {
                text: qsTr("Cancel")
                focus: true
                onClicked: uninstallDialog.close()
            },
            Item { Layout.fillWidth: true },
            Button {
                variant: "danger"
                text: qsTr("Uninstall")
                onClicked: {
                    const finish = () => {
                        Settings.set("startup.atLogin", false)
                        const note = Launcher.uninstall()
                        Daemon.call("daemon.quit", {})
                        if (note.length > 0)
                            uninstallNote.text = note
                        else
                            Qt.quit()
                    }
                    const steps = []
                    if (page.deleteHistory)
                        steps.push(done => Daemon.call("data.wipe", { confirm: "DELETE" }, done))
                    if (page.deleteModels)
                        steps.push(done => Daemon.call("models.list", {}, (ok, r) => {
                            const ids = ok ? r.installed.map(m => m.id) : []
                            let left = ids.length
                            if (left === 0)
                                done()
                            for (const id of ids)
                                Daemon.call("models.delete", { id: id }, () => { if (--left === 0) done() })
                        }))
                    const next = () => steps.length > 0 ? steps.shift()(next) : finish()
                    next()
                }
            }
        ]
    }

    Section {
        Layout.fillWidth: true
        title: Brand.fullName
        Repeater {
            model: [
                { name: qsTr("Version"), value: Brand.version + (Brand.build.length > 0 ? " (" + Brand.build + ")" : "") },
                { name: qsTr("Background service"), value: Daemon.daemonVersion || qsTr("Not connected") },
                { name: qsTr("Model manifest"), value: Daemon.manifestVersion > 0 ? String(Daemon.manifestVersion) : qsTr("Unknown") },
                { name: qsTr("Interface toolkit"), value: "Qt " + (page.licenses ? (page.licenses.components.find(c => c.name === "Qt") || {}).version || "" : "") }
            ]
            RowLayout {
                required property var modelData
                Layout.fillWidth: true
                spacing: Theme.s3
                Accessible.role: Accessible.StaticText
                Accessible.name: modelData.name + ": " + modelData.value
                Label {
                    Layout.preferredWidth: Theme.px(180)
                    text: modelData.name
                    color: Theme.textSecondary
                    Accessible.ignored: true
                }
                Label {
                    Layout.fillWidth: true
                    text: modelData.value
                    role: "mono"
                    Accessible.ignored: true
                }
            }
        }
        Flow {
            Layout.fillWidth: true
            spacing: Theme.s2
            Button {
                text: qsTr("License")
                iconName: "file-text"
                onClicked: page.showText(qsTr("%1 license").arg(Brand.productName), Platform.resourceText("licenses/setu.txt"))
            }
            Button {
                text: qsTr("Release notes")
                iconName: "file-text"
                onClicked: page.showText(qsTr("Release notes"), Platform.resourceText("licenses/release-notes.txt"))
            }
            Button {
                visible: Brand.supportUrl.length > 0
                text: qsTr("Support")
                iconName: "external-link"
                onClicked: Platform.openUrl(Brand.supportUrl)
            }
            Button {
                visible: Brand.feedbackAddress.length > 0
                text: qsTr("Send feedback")
                iconName: "mail"
                onClicked: Platform.openUrl("mailto:" + Brand.feedbackAddress)
            }
        }
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("Third-party licenses")
        description: qsTr("%1 is built with these components. Fonts and models are listed too.").arg(Brand.productName)
        Component.onCompleted: page.loadLicenses()
        SearchField {
            id: filter
            objectName: "setting:licenses"
            Layout.fillWidth: true
            label: qsTr("Search licenses")
        }
        Label {
            role: "small"
            color: Theme.textSecondary
            text: qsTr("%n components", "", page.matches.length)
        }
        RowList {
            Layout.fillWidth: true
            maxVisible: 10
            label: qsTr("Third-party licenses")
            model: page.matches
            emptyText: qsTr("No component matches \"%1\".").arg(filter.text)
            delegate: ListRow {
                required property var modelData
                width: ListView.view.width
                title: modelData.name + (modelData.version ? "  " + modelData.version : "")
                secondary: modelData.license + "  ·  " + modelData.kind
                onClicked: {
                    const texts = (modelData.texts || []).map(id => page.licenses.texts[id]).filter(t => t)
                    page.showText(modelData.name, [modelData.license, modelData.url].filter(x => x).join("\n") + "\n\n" + (texts.length > 0 ? texts.join("\n\n----------------------------------------\n\n") : qsTr("The license named above applies. Its text is published at the address above.")))
                }
            }
        }
    }

    Section {
        Layout.fillWidth: true
        title: qsTr("Uninstall")
        Button {
            text: qsTr("Uninstall %1").arg(Brand.productName)
            variant: "danger"
            onClicked: Daemon.call("models.list", {}, (ok, r) => {
                page.modelBytes = ok ? r.diskUsedBytes : 0
                uninstallDialog.open()
            })
        }
    }
}
