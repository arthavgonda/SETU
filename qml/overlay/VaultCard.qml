import QtQuick
import QtQuick.Layouts
import QtQuick.Window
import Setu.Core
import Setu.Theme
import Ledger

Window {
    id: win
    property var entry: ({})
    property string problem: ""
    property bool busy: false
    property bool reveal: false
    readonly property string mode: entry.mode || "save"
    readonly property bool asksName: mode === "save"
    readonly property bool calm: Motion.reduced

    width: Theme.px(420)
    height: col.implicitHeight + Theme.px(48)
    x: Screen.virtualX + (Screen.width - width) / 2
    y: Screen.virtualY + Math.max(Theme.px(80), (Screen.height - height) / 3)
    visible: true
    flags: Qt.Dialog | Qt.WindowStaysOnTopHint
    color: Theme.notchBg
    title: mode === "system" ? qsTr("Your system password") : mode === "confirm" ? qsTr("Confirm it's you") : qsTr("Save a password")
    onClosing: close => {
        if (!win.busy)
            Daemon.call("vault.cancel", { id: win.entry.id })
    }
    onEntryChanged: {
        if (entry.error)
            problem = entry.error
        if (asksName && name.text.length === 0)
            name.text = entry.title || ""
        if (asksName && account.text.length === 0)
            account.text = entry.account || ""
    }
    Component.onCompleted: {
        requestActivate()
        Qt.callLater(() => (asksName && name.text.length === 0 ? name : secret).forceActiveFocus())
    }

    function submit() {
        if (busy)
            return
        if (secret.text.length === 0) {
            problem = qsTr("Type the password first.")
            secret.forceActiveFocus()
            return
        }
        busy = true
        problem = ""
        const p = { id: entry.id, password: secret.text }
        if (asksName) {
            p.title = name.text
            p.account = account.text
        }
        Daemon.call("vault.submit", p, (ok, r, e) => {
            busy = false
            if (!ok) {
                problem = e && e.message ? e.message : qsTr("That didn't work.")
                if (mode !== "save")
                    secret.text = ""
                secret.forceActiveFocus()
            }
        })
    }

    component Field: Rectangle {
        id: box
        property alias input: input
        property string hint
        property string label
        Layout.fillWidth: true
        Layout.preferredHeight: Theme.px(46)
        radius: Theme.px(12)
        color: Theme.notchCard
        border.width: input.activeFocus ? 2 : 1
        border.color: input.activeFocus ? Theme.notchListenDeep : Theme.notchRaised
        TextInput {
            id: input
            anchors.fill: parent
            anchors.leftMargin: Theme.px(14)
            anchors.rightMargin: Theme.px(14)
            verticalAlignment: TextInput.AlignVCenter
            color: Theme.notchInk
            selectionColor: Theme.notchListenDeep
            selectedTextColor: Theme.notchInk
            font.family: Type.sans
            font.pixelSize: Theme.px(16)
            clip: true
            activeFocusOnTab: true
            Accessible.role: Accessible.EditableText
            Accessible.name: box.label
            Accessible.passwordEdit: echoMode !== TextInput.Normal
            Keys.onReturnPressed: win.submit()
            Keys.onEnterPressed: win.submit()
            Keys.onEscapePressed: win.close()
            Text {
                anchors.verticalCenter: parent.verticalCenter
                visible: input.text.length === 0
                text: box.hint
                color: Theme.notchFaint
                font: input.font
                Accessible.ignored: true
            }
        }
    }

    component Pill: Rectangle {
        id: pill
        property string label
        property bool primary: false
        signal clicked
        Layout.preferredHeight: Theme.px(40)
        Layout.preferredWidth: Math.max(Theme.px(110), txt.implicitWidth + Theme.px(32))
        radius: height / 2
        color: primary ? (tap.pressed ? Theme.notchMuted : Theme.notchInk) : (tap.pressed ? Theme.notchRaised : Theme.notchCard)
        opacity: enabled ? 1 : 0.5
        Accessible.role: Accessible.Button
        Accessible.name: label
        Accessible.onPressAction: pill.clicked()
        Text {
            id: txt
            anchors.centerIn: parent
            text: pill.label
            color: pill.primary ? Theme.notchBg : Theme.notchInk
            font.family: Type.sans
            font.pixelSize: Theme.px(14)
            font.weight: Font.DemiBold
        }
        TapHandler {
            id: tap
            enabled: pill.enabled
            onTapped: pill.clicked()
        }
        HoverHandler {
            cursorShape: Qt.PointingHandCursor
        }
    }

    Rectangle {
        anchors.fill: parent
        color: Theme.notchBg

        ColumnLayout {
            id: col
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.margins: Theme.px(24)
            spacing: Theme.px(12)

            Text {
                Layout.fillWidth: true
                text: win.title
                color: Theme.notchInk
                font.family: Type.sans
                font.pixelSize: Theme.px(19)
                font.weight: Font.DemiBold
            }
            Text {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                text: win.entry.message || ""
                color: Theme.notchMuted
                font.family: Type.sans
                font.pixelSize: Theme.px(13)
            }
            Field {
                id: nameBox
                visible: win.asksName
                label: qsTr("Name")
                hint: qsTr("Name, like Google or Bank")
                input.maximumLength: 80
            }
            Field {
                id: accountBox
                visible: win.asksName
                label: qsTr("Account")
                hint: qsTr("Email or user name (optional)")
                input.maximumLength: 120
                input.inputMethodHints: Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText
            }
            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.px(8)
                Field {
                    id: secretBox
                    label: qsTr("Password")
                    hint: qsTr("Password")
                    input.echoMode: win.reveal ? TextInput.Normal : TextInput.Password
                    input.passwordCharacter: "•"
                    input.maximumLength: 1024
                    input.inputMethodHints: Qt.ImhHiddenText | Qt.ImhSensitiveData | Qt.ImhNoPredictiveText | Qt.ImhNoAutoUppercase
                }
                Pill {
                    label: win.reveal ? qsTr("Hide") : qsTr("Show")
                    Layout.preferredWidth: Theme.px(72)
                    onClicked: win.reveal = !win.reveal
                }
            }
            Text {
                Layout.fillWidth: true
                visible: win.problem.length > 0
                wrapMode: Text.WordWrap
                text: win.problem
                color: Theme.notchDanger
                font.family: Type.sans
                font.pixelSize: Theme.px(13)
            }
            RowLayout {
                Layout.fillWidth: true
                Layout.topMargin: Theme.px(4)
                spacing: Theme.px(10)
                Item { Layout.fillWidth: true }
                Pill {
                    label: qsTr("Cancel")
                    onClicked: win.close()
                }
                Pill {
                    primary: true
                    enabled: !win.busy
                    label: win.busy ? qsTr("Checking") : win.mode === "confirm" ? qsTr("Continue") : qsTr("Save")
                    onClicked: win.submit()
                }
            }
        }
    }

    readonly property var name: nameBox.input
    readonly property var account: accountBox.input
    readonly property var secret: secretBox.input
}
