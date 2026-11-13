import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

FocusScope {
    id: page
    property int chat: -1
    property var messages: []
    property bool waiting: false
    property string waitingText
    property string pendingInteraction
    property bool loadingChat: false
    property string chatTitle
    property string streaming
    signal chatOpened(int id)
    signal navigate(string page)
    signal toast(string text)

    readonly property bool empty: messages.length === 0 && !waiting && !loadingChat
    readonly property int column: Math.min(width - Theme.s6 * 2, Theme.px(760))
    readonly property int textSize: Math.round(15 * Theme.textScale)
    readonly property string firstName: Platform.userName.split(" ")[0]
    readonly property string greeting: {
        const h = new Date().getHours()
        if (h < 5)
            return qsTr("Still up, %1?").arg(firstName)
        if (h < 12)
            return qsTr("Good morning, %1").arg(firstName)
        if (h < 17)
            return qsTr("Good afternoon, %1").arg(firstName)
        return qsTr("Good evening, %1").arg(firstName)
    }
    readonly property var ideas: [
        qsTr("Open YouTube and play lo-fi music"),
        qsTr("Set a timer for 10 minutes"),
        qsTr("Create a new Python project called weather app"),
        qsTr("Explain how a rainbow forms")
    ]

    function reset() {
        chat = -1
        messages = []
        waiting = false
        streaming = ""
        pendingInteraction = ""
        chatTitle = ""
        Daemon.call("chat.focus", {})
        Qt.callLater(composer.focusInput)
    }

    function load(id) {
        if (id === chat && messages.length > 0)
            return
        chat = id
        messages = []
        waiting = false
        loadingChat = true
        Daemon.call("chat.focus", { id: id })
        Daemon.call("chat.get", { id: id }, (ok, r) => {
            page.loadingChat = false
            if (!ok || page.chat !== id)
                return
            page.messages = r.messages
            page.chatTitle = r.chat ? r.chat.title : ""
            Qt.callLater(page.toEnd)
        })
        Qt.callLater(composer.focusInput)
    }

    function send(text) {
        const t = text.trim()
        if (t.length === 0)
            return
        Daemon.call("sound.play", { name: "tap" })
        waiting = true
        waitingText = qsTr("Thinking")
        if (chat < 0)
            messages = messages.concat([{ id: -Date.now(), role: "user", text: t, local: true }])
        Daemon.call("chat.send", { chat: chat >= 0 ? chat : null, text: t }, (ok, r, e) => {
            if (!ok) {
                page.waiting = false
                page.toast(e.message)
                return
            }
            if (page.chat !== r.chat) {
                page.chat = r.chat
                page.chatOpened(r.chat)
                Daemon.call("chat.get", { id: r.chat }, (ok2, r2) => {
                    if (ok2 && page.chat === r.chat) {
                        page.messages = r2.messages
                        page.chatTitle = r2.chat ? r2.chat.title : ""
                        Qt.callLater(page.toEnd)
                    }
                })
            }
            page.pendingInteraction = r.interaction ? String(r.interaction) : ""
            if (r.error)
                page.waiting = false
        })
        Qt.callLater(toEnd)
    }

    function talk() {
        Daemon.call("session.listen", { chat: chat >= 0 ? chat : null }, (ok, r, e) => {
            if (!ok)
                page.toast(e.message)
        })
    }

    function toEnd() {
        list.positionViewAtEnd()
    }
    function pageList(dir) {
        const top = list.originY
        const bottom = list.originY + Math.max(0, list.contentHeight - list.height)
        list.contentY = Math.max(top, Math.min(bottom, list.contentY + dir * list.height * 0.9))
    }

    Subscription {
        topics: ["chat.message", "chat.delta", "chat.changed", "action.started", "interaction.ended"]
        onReceived: (topic, d) => {
            if (topic === "chat.message") {
                if (page.chat < 0 && d.role === "user" && !page.waiting) {
                    page.chat = d.chat
                    page.chatOpened(d.chat)
                }
                if (d.chat !== page.chat)
                    return
                let list = page.messages.filter(m => !(m.local && m.role === d.role && m.text === d.text))
                if (!list.some(m => m.id === d.id))
                    list.push(d)
                page.messages = list
                if (d.role === "assistant") {
                    page.waiting = false
                    page.streaming = ""
                }
                Qt.callLater(page.toEnd)
            } else if (topic === "chat.delta") {
                if (d.chat === page.chat && page.waiting) {
                    page.streaming = d.text
                    Qt.callLater(page.toEnd)
                }
            } else if (topic === "chat.changed") {
                if (d.chat && d.chat.id === page.chat)
                    page.chatTitle = d.chat.title
            } else if (topic === "action.started") {
                if (page.waiting && String(d.interaction) === page.pendingInteraction && d.label)
                    page.waitingText = d.label
            } else if (topic === "interaction.ended") {
                if (String(d.interaction) === page.pendingInteraction)
                    endTimer.restart()
            }
        }
    }

    Timer {
        id: endTimer
        interval: 1500
        onTriggered: {
            page.waiting = false
            page.streaming = ""
        }
    }

    Item {
        anchors.fill: parent
        visible: !page.empty

        ListView {
            id: list
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.bottom: dock.top
            model: page.messages
            clip: true
            spacing: Theme.s4
            boundsBehavior: Flickable.StopAtBounds
            activeFocusOnTab: true
            Accessible.role: Accessible.List
            Accessible.name: page.chatTitle.length > 0 ? page.chatTitle : qsTr("Chat")
            ScrollBar.vertical: ScrollBar {}
            header: Item {
                width: list.width
                height: Theme.s6
            }
            footer: Item {
                width: list.width
                height: typing.visible ? typing.implicitHeight + Theme.s6 : Theme.s5
                RowLayout {
                    id: typing
                    visible: page.waiting
                    x: (list.width - page.column) / 2
                    width: page.column
                    spacing: Theme.s3
                    SetuFace {
                        Layout.preferredWidth: Theme.px(34)
                        Layout.preferredHeight: Theme.px(20)
                        Layout.alignment: Qt.AlignTop
                        mood: "think"
                        Accessible.ignored: true
                    }
                    TextEdit {
                        Layout.fillWidth: true
                        visible: page.streaming.length > 0
                        text: page.streaming
                        textFormat: TextEdit.PlainText
                        wrapMode: TextEdit.Wrap
                        readOnly: true
                        color: Theme.textPrimary
                        font.family: Type.sans
                        font.pixelSize: page.textSize
                        Accessible.role: Accessible.StaticText
                        Accessible.name: text
                    }
                    Row {
                        visible: page.streaming.length === 0
                        spacing: Theme.px(4)
                        Repeater {
                            model: 3
                            Rectangle {
                                id: dot
                                required property int index
                                width: Theme.px(6)
                                height: width
                                radius: width / 2
                                color: Theme.textSecondary
                                opacity: 0.35
                                SequentialAnimation on opacity {
                                    running: typing.visible && !Motion.reduced
                                    loops: Animation.Infinite
                                    PauseAnimation { duration: dot.index * 140 }
                                    NumberAnimation { from: 0.3; to: 1; duration: 260 }
                                    NumberAnimation { from: 1; to: 0.3; duration: 260 }
                                    PauseAnimation { duration: (2 - dot.index) * 140 }
                                }
                            }
                        }
                    }
                    Label {
                        Layout.fillWidth: true
                        visible: page.streaming.length === 0
                        text: page.waitingText
                        color: Theme.textSecondary
                        role: "small"
                    }
                }
            }
            delegate: Item {
                id: msg
                required property var modelData
                required property int index
                readonly property bool mine: modelData.role === "user"
                width: list.width
                height: bubble.height
                opacity: 0
                Component.onCompleted: appear.start()
                NumberAnimation {
                    id: appear
                    target: msg
                    property: "opacity"
                    to: 1
                    duration: Motion.fade
                    easing.type: Easing.OutCubic
                }
                Item {
                    id: bubble
                    x: (list.width - page.column) / 2
                    width: page.column
                    height: msg.mine ? mineBox.height : theirs.implicitHeight
                    Rectangle {
                        id: mineBox
                        visible: msg.mine
                        anchors.right: parent.right
                        width: Math.min(page.column * 0.78, mineText.implicitWidth + Theme.s4 * 2)
                        height: mineText.implicitHeight + Theme.s3 * 2
                        radius: Theme.px(20)
                        color: Theme.bgSelected
                        border.width: Theme.contrast ? Theme.border : 0
                        border.color: Theme.borderStrong
                        TextEdit {
                            id: mineText
                            x: Theme.s4
                            y: Theme.s3
                            width: Math.min(page.column * 0.78 - Theme.s4 * 2, implicitWidth)
                            text: msg.modelData.text
                            textFormat: TextEdit.PlainText
                            wrapMode: TextEdit.Wrap
                            readOnly: true
                            selectByMouse: true
                            color: Theme.textPrimary
                            selectionColor: Theme.accent
                            selectedTextColor: Theme.textOnAccent
                            font.family: Type.sans
                            font.pixelSize: page.textSize
                            Accessible.role: Accessible.StaticText
                            Accessible.name: qsTr("You said: %1").arg(text)
                        }
                    }
                    RowLayout {
                        id: theirs
                        visible: !msg.mine
                        width: parent.width
                        spacing: Theme.s3
                        SetuFace {
                            Layout.preferredWidth: Theme.px(34)
                            Layout.preferredHeight: Theme.px(20)
                            Layout.alignment: Qt.AlignTop
                            Layout.topMargin: Theme.px(2)
                            mood: "happy"
                            Accessible.ignored: true
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: Theme.s1
                            TextEdit {
                                id: answer
                                Layout.fillWidth: true
                                text: msg.modelData.text
                                textFormat: TextEdit.PlainText
                                wrapMode: TextEdit.Wrap
                                readOnly: true
                                selectByMouse: true
                                color: Theme.textPrimary
                                selectionColor: Theme.accent
                                selectedTextColor: Theme.textOnAccent
                                font.family: Type.sans
                                font.pixelSize: page.textSize
                                Accessible.role: Accessible.StaticText
                                Accessible.name: qsTr("%1 said: %2").arg(Brand.productName).arg(text)
                            }
                            IconButton {
                                label: qsTr("Copy")
                                iconName: "copy"
                                ink: Theme.textTertiary
                                implicitWidth: Theme.hit
                                implicitHeight: Theme.hit
                                opacity: msgHover.hovered || activeFocus ? 1 : 0
                                onClicked: {
                                    Platform.copy(msg.modelData.text)
                                    page.toast(qsTr("Copied"))
                                }
                            }
                        }
                    }
                }
                HoverHandler {
                    id: msgHover
                }
            }
        }

        Item {
            id: dock
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: dockComposer.implicitHeight + Theme.s5
            Rectangle {
                anchors.fill: parent
                color: Theme.bgBase
            }
            ChatComposer {
                id: dockComposer
                x: (parent.width - page.column) / 2
                width: page.column
                y: 0
                busy: page.waiting
                onSend: text => page.send(text)
                onTalk: page.talk()
                onPageRequested: dir => page.pageList(dir)
            }
        }
    }

    Flickable {
        id: helloScroll
        anchors.fill: parent
        visible: page.empty
        contentWidth: width
        contentHeight: Math.max(height, hello.implicitHeight + Theme.s6 * 2)
        boundsBehavior: Flickable.StopAtBounds
        interactive: contentHeight > height
        clip: true
        ScrollBar.vertical: ScrollBar {}

        ColumnLayout {
            id: hello
            x: (helloScroll.width - width) / 2
            y: Math.max(Theme.s6, (helloScroll.contentHeight - implicitHeight) / 2)
            width: page.column
            spacing: Theme.s5

            SetuFace {
                Layout.alignment: Qt.AlignHCenter
                Layout.preferredWidth: Theme.px(112)
                Layout.preferredHeight: Theme.px(56)
                mood: Daemon.mic.muted === true ? "sleepy" : Daemon.session.state === "capturing" ? "listen" : "happy"
                Accessible.ignored: true
            }
            Label {
                Layout.fillWidth: true
                horizontalAlignment: Text.AlignHCenter
                text: page.greeting
                role: "display"
                Accessible.role: Accessible.Heading
            }
            ChatComposer {
                id: composer
                Layout.fillWidth: true
                busy: page.waiting
                onSend: text => page.send(text)
                onTalk: page.talk()
            }
            Flow {
                Layout.fillWidth: true
                spacing: Theme.s2
                Repeater {
                    model: page.ideas
                    Chip {
                        required property string modelData
                        text: modelData
                        onClicked: page.send(modelData)
                    }
                }
            }
            Label {
                Layout.fillWidth: true
                horizontalAlignment: Text.AlignHCenter
                role: "small"
                color: Theme.textTertiary
                text: Daemon.mic.muted === true ? qsTr("The microphone is off.") : qsTr("Or just say “%1”.").arg(Brand.wakePhrase)
            }
        }
    }
}
