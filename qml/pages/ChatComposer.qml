import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

Rectangle {
    id: box
    property alias text: input.text
    property bool busy: false
    property string placeholder: qsTr("Message %1").arg(Brand.productName)
    readonly property bool canSend: input.text.trim().length > 0 && !busy && Daemon.connected
    readonly property bool listening: Daemon.session.state === "capturing"
    signal send(string text)
    signal talk
    signal pageRequested(int dir)
    implicitHeight: Math.max(Theme.px(56), column.implicitHeight + Theme.s2 * 2)
    radius: Theme.px(26)
    color: Theme.bgRaised
    border.width: Theme.border
    border.color: input.activeFocus ? Theme.borderStrong : Theme.borderSubtle
    Behavior on border.color {
        ColorAnimation { duration: Motion.micro }
    }

    function submit() {
        if (!canSend)
            return
        const t = input.text.trim()
        input.text = ""
        box.send(t)
    }

    function focusInput() {
        input.forceActiveFocus()
    }

    Rectangle {
        anchors.fill: parent
        anchors.margins: -Theme.px(6)
        z: -1
        radius: box.radius + Theme.px(6)
        color: Theme.glow
        visible: input.activeFocus && !Theme.contrast
    }

    RowLayout {
        id: column
        anchors.fill: parent
        anchors.leftMargin: Theme.s2
        anchors.rightMargin: Theme.s2
        anchors.topMargin: Theme.s2
        anchors.bottomMargin: Theme.s2
        spacing: Theme.s2

        IconButton {
            Layout.alignment: Qt.AlignBottom
            label: box.listening ? qsTr("Listening") : qsTr("Talk to %1").arg(Brand.productName)
            iconName: Daemon.mic.muted === true ? "mic-off" : "mic"
            ink: box.listening ? Theme.notchListenDeep : Theme.textSecondary
            enabled: Daemon.connected && Daemon.mic.muted !== true
            onClicked: box.talk()
        }

        Flickable {
            id: flick
            Layout.fillWidth: true
            Layout.alignment: Qt.AlignVCenter
            Layout.preferredHeight: Math.min(input.implicitHeight, Theme.px(180))
            contentWidth: width
            contentHeight: input.implicitHeight
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            interactive: contentHeight > height
            function ensureVisible(r) {
                if (contentY >= r.y)
                    contentY = r.y
                else if (contentY + height <= r.y + r.height)
                    contentY = r.y + r.height - height
            }
            TextEdit {
                id: input
                width: flick.width
                wrapMode: TextEdit.Wrap
                textFormat: TextEdit.PlainText
                color: Theme.textPrimary
                selectionColor: Theme.accent
                selectedTextColor: Theme.textOnAccent
                font.family: Type.sans
                font.pixelSize: Type.body.size
                padding: Theme.s1
                selectByMouse: true
                persistentSelection: true
                activeFocusOnTab: true
                Accessible.role: Accessible.EditableText
                Accessible.name: box.placeholder
                property real attended: 0
                function attend() {
                    const now = Date.now()
                    if (Daemon.connected && now - attended > 4000) {
                        attended = now
                        Daemon.call("session.attend", {}, (ok, r, e) => {})
                    }
                }
                onActiveFocusChanged: if (activeFocus) attend()
                onTextChanged: if (text.length > 0) attend()
                onCursorRectangleChanged: flick.ensureVisible(cursorRectangle)
                Keys.onPressed: event => {
                    if ((event.key === Qt.Key_Return || event.key === Qt.Key_Enter) && !(event.modifiers & Qt.ShiftModifier)) {
                        box.submit()
                        event.accepted = true
                    } else if (event.key === Qt.Key_PageDown || event.key === Qt.Key_PageUp) {
                        box.pageRequested(event.key === Qt.Key_PageDown ? 1 : -1)
                        event.accepted = true
                    }
                }
                Text {
                    anchors.fill: parent
                    anchors.margins: input.padding
                    visible: input.text.length === 0 && !input.inputMethodComposing
                    text: box.placeholder
                    color: Theme.textTertiary
                    font: input.font
                    elide: Text.ElideRight
                    Accessible.ignored: true
                }
            }
        }

        Rectangle {
            id: sendButton
            Layout.alignment: Qt.AlignBottom
            implicitWidth: Theme.px(36)
            implicitHeight: Theme.px(36)
            radius: width / 2
            color: box.canSend ? (sendTap.pressed ? Theme.accentPressed : (sendHover.hovered ? Theme.accentHover : Theme.accent)) : Theme.bgPressed
            activeFocusOnTab: true
            Accessible.role: Accessible.Button
            Accessible.name: qsTr("Send")
            Accessible.onPressAction: box.submit()
            Keys.onReturnPressed: box.submit()
            Keys.onSpacePressed: box.submit()
            scale: sendTap.pressed && box.canSend ? 0.92 : 1
            Behavior on scale {
                NumberAnimation { duration: Motion.micro; easing.type: Easing.OutQuad }
            }
            Behavior on color {
                ColorAnimation { duration: Motion.micro }
            }
            Rectangle {
                anchors.fill: parent
                anchors.margins: -Theme.focusGap - Theme.focusWidth
                radius: width / 2
                color: "transparent"
                border.width: Theme.focusWidth
                border.color: Theme.accent
                visible: sendButton.activeFocus
            }
            Icon {
                anchors.centerIn: parent
                name: box.busy ? "square" : "arrow-up"
                color: box.canSend ? Theme.textOnAccent : Theme.textTertiary
            }
            HoverHandler {
                id: sendHover
                cursorShape: box.canSend ? Qt.PointingHandCursor : Qt.ArrowCursor
            }
            TapHandler {
                id: sendTap
                onTapped: box.submit()
            }
        }
    }
}
