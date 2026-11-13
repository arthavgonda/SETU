import QtQuick
import QtQuick.Layouts
import QtQuick.Window
import QtQuick.Dialogs
import Setu.Core
import Setu.Theme
import Ledger

Window {
    id: win
    property var look: ({})
    property string problem: ""
    property var picked: []
    property string mainDir: ""
    property bool seeded: false
    readonly property string step: look.step || "look"
    readonly property var avatars: look.avatars || []
    readonly property var backgrounds: look.backgrounds || []
    readonly property var profiles: look.profiles || []
    readonly property int avatarIndex: Math.max(0, avatars.findIndex(a => a.id === look.avatar))
    readonly property int backgroundIndex: Math.max(0, backgrounds.findIndex(b => b.id === look.background))
    readonly property bool firefox: look.family === "firefox"
    readonly property bool calm: Motion.reduced
    signal closed

    width: Theme.px(720)
    height: Theme.px(600)
    minimumWidth: Theme.px(560)
    minimumHeight: Theme.px(520)
    x: Screen.virtualX + (Screen.width - width) / 2
    y: Screen.virtualY + (Screen.height - height) / 2
    visible: true
    color: Theme.notchBg
    title: look.edit ? qsTr("Your Setu profile") : qsTr("New Setu profile for %1").arg(look.display || "")
    onClosing: close => {
        if (win.step !== "making")
            win.act("cancel")
    }

    function avatarUrl(id) {
        return id ? "qrc:/looks/avatars/" + id + ".png" : ""
    }
    function backgroundUrl(id, thumb) {
        return id ? "qrc:/looks/" + (thumb ? "thumbs/" : "backgrounds/") + id + ".jpg" : ""
    }
    function act(op, extra) {
        win.problem = ""
        Daemon.call("look.act", Object.assign({ id: win.look.id, op: op }, extra || {}), (ok, r, e) => {
            if (!ok && e)
                win.problem = e.message
        })
    }
    function move(op) {
        Daemon.call("sound.play", { name: "tap" })
        act(op)
    }
    function toggle(dir) {
        if (win.firefox) {
            win.picked = [dir]
            win.mainDir = dir
            return
        }
        const i = win.picked.indexOf(dir)
        win.picked = i >= 0 ? win.picked.filter(d => d !== dir) : win.picked.concat([dir])
        if (win.picked.indexOf(win.mainDir) < 0)
            win.mainDir = win.picked.length > 0 ? win.picked[0] : ""
    }
    onProfilesChanged: {
        if (!win.seeded && win.profiles.length > 0) {
            win.seeded = true
            const cur = win.profiles.find(p => p.current) || win.profiles[0]
            win.picked = win.firefox ? [cur.dir] : win.profiles.map(p => p.dir)
            win.mainDir = cur.dir
        }
    }
    onStepChanged: {
        win.problem = ""
        if (step === "name")
            Qt.callLater(() => input.forceActiveFocus())
        else if (step === "look")
            Qt.callLater(() => stage.forceActiveFocus())
    }
    Component.onCompleted: {
        requestActivate()
        stage.forceActiveFocus()
    }

    FileDialog {
        id: pictureDialog
        title: qsTr("Choose your own picture")
        nameFilters: [qsTr("Pictures (*.png *.jpg *.jpeg *.webp *.bmp *.gif)")]
        onAccepted: win.act("custom", { value: Platform.localFile(selectedFile) })
    }

    component Pill: Rectangle {
        id: pill
        property string text
        property bool solid: false
        property bool busy: false
        signal pressed
        implicitWidth: label.implicitWidth + Theme.px(36)
        implicitHeight: Theme.px(44)
        radius: height / 2
        color: solid ? (pillHover.hovered ? Theme.lookTileHover : Theme.notchInk) : (pillHover.hovered ? Theme.notchRaised : Theme.notchCard)
        border.width: activeFocus ? 2 : (solid ? 0 : 1)
        border.color: activeFocus ? (solid ? Theme.notchListenDeep : Theme.notchInk) : Theme.notchRaised
        opacity: enabled ? 1 : 0.4
        activeFocusOnTab: true
        scale: pillTap.pressed && !win.calm ? 0.96 : 1
        Behavior on scale {
            NumberAnimation { duration: 120 }
        }
        Accessible.role: Accessible.Button
        Accessible.name: text
        Accessible.onPressAction: pill.pressed()
        Keys.onReturnPressed: pill.pressed()
        Keys.onEnterPressed: pill.pressed()
        Keys.onSpacePressed: pill.pressed()
        Text {
            id: label
            anchors.centerIn: parent
            text: pill.busy ? qsTr("Working…") : pill.text
            color: pill.solid ? Theme.notchBg : Theme.notchInk
            font.family: Type.sans
            font.pixelSize: Theme.px(15)
            font.weight: Font.DemiBold
            Accessible.ignored: true
        }
        HoverHandler {
            id: pillHover
            cursorShape: Qt.PointingHandCursor
        }
        TapHandler {
            id: pillTap
            onTapped: if (pill.enabled) pill.pressed()
        }
    }

    component Arrow: Rectangle {
        id: arrow
        property bool backward: true
        property string name
        signal pressed
        width: Theme.px(44)
        height: width
        radius: width / 2
        color: arrowHover.hovered ? Theme.lookArrowHover : Theme.lookArrow
        border.width: activeFocus ? 2 : 1
        border.color: activeFocus ? Theme.notchInk : Theme.lookEdge
        activeFocusOnTab: true
        Accessible.role: Accessible.Button
        Accessible.name: name
        Accessible.onPressAction: arrow.pressed()
        Keys.onReturnPressed: arrow.pressed()
        Keys.onSpacePressed: arrow.pressed()
        Icon {
            anchors.centerIn: parent
            name: arrow.backward ? "chevron-left" : "chevron-right"
            size: Theme.px(22)
            color: Theme.notchInk
        }
        HoverHandler {
            id: arrowHover
            cursorShape: Qt.PointingHandCursor
        }
        TapHandler {
            onTapped: arrow.pressed()
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        Item {
            id: stage
            Layout.fillWidth: true
            Layout.preferredHeight: win.step === "look" ? Theme.px(300) : Theme.px(220)
            clip: true
            focus: true
            Accessible.role: Accessible.Pane
            Accessible.name: qsTr("Profile picture %1 on the %2 background").arg(win.look.avatarLabel || "").arg((win.backgrounds[win.backgroundIndex] || {}).label || "")
            Behavior on Layout.preferredHeight {
                NumberAnimation { duration: win.calm ? 0 : 260; easing.type: Easing.OutCubic }
            }
            Keys.onLeftPressed: event => win.move(event.modifiers & Qt.ShiftModifier ? "bg_prev" : "prev")
            Keys.onRightPressed: event => win.move(event.modifiers & Qt.ShiftModifier ? "bg_next" : "next")
            Keys.onReturnPressed: if (win.step === "look") win.act("confirm")
            Keys.onEnterPressed: if (win.step === "look") win.act("confirm")
            Keys.onEscapePressed: win.act("cancel")

            Image {
                id: backdrop
                anchors.fill: parent
                source: win.backgroundUrl(win.look.background, false)
                fillMode: Image.PreserveAspectCrop
                asynchronous: true
                smooth: true
                opacity: status === Image.Ready ? 1 : 0
                Behavior on opacity {
                    NumberAnimation { duration: win.calm ? 0 : 280 }
                }
                Accessible.ignored: true
            }
            Rectangle {
                anchors.fill: parent
                gradient: Gradient {
                    GradientStop { position: 0.0; color: Theme.lookShadeTop }
                    GradientStop { position: 0.65; color: Theme.lookShadeBottom }
                    GradientStop { position: 1.0; color: Theme.notchBg }
                }
            }
            Item {
                id: ring
                anchors.horizontalCenter: parent.horizontalCenter
                anchors.verticalCenter: parent.verticalCenter
                anchors.verticalCenterOffset: -Theme.px(10)
                width: win.step === "look" ? Theme.px(176) : Theme.px(132)
                height: width
                Behavior on width {
                    NumberAnimation { duration: win.calm ? 0 : 260; easing.type: Easing.OutCubic }
                }
                Rectangle {
                    anchors.centerIn: parent
                    width: parent.width + Theme.px(10)
                    height: width
                    radius: width / 2
                    color: Theme.notchInk
                    opacity: 0.95
                }
                LookAvatar {
                    id: face
                    anchors.fill: parent
                    avatar: win.look.avatar || ""
                    file: win.look.custom || ""
                    scale: 1
                    SequentialAnimation on scale {
                        id: pop
                        running: false
                        NumberAnimation { to: 0.88; duration: win.calm ? 0 : 90 }
                        NumberAnimation { to: 1; duration: win.calm ? 0 : 320; easing.type: Easing.OutBack; easing.overshoot: 2 }
                    }
                }
                Connections {
                    target: win
                    function onAvatarIndexChanged() { pop.restart() }
                }
            }
            Arrow {
                visible: win.step === "look"
                anchors.verticalCenter: ring.verticalCenter
                anchors.right: ring.left
                anchors.rightMargin: Theme.px(28)
                backward: true
                name: qsTr("Previous picture")
                onPressed: win.move("prev")
            }
            Arrow {
                visible: win.step === "look"
                anchors.verticalCenter: ring.verticalCenter
                anchors.left: ring.right
                anchors.leftMargin: Theme.px(28)
                backward: false
                name: qsTr("Next picture")
                onPressed: win.move("next")
            }
            Text {
                anchors.horizontalCenter: parent.horizontalCenter
                anchors.top: ring.bottom
                anchors.topMargin: Theme.px(14)
                text: win.step === "look" ? (win.look.avatarLabel || "") : (win.look.finalName || win.look.name || "")
                color: Theme.notchInk
                style: Text.Raised
                styleColor: Theme.lookTextShadow
                font.family: Type.sans
                font.pixelSize: Theme.px(20)
                font.weight: Font.DemiBold
                textFormat: Text.PlainText
            }
        }

        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            ColumnLayout {
                anchors.fill: parent
                anchors.leftMargin: Theme.px(24)
                anchors.rightMargin: Theme.px(24)
                anchors.bottomMargin: Theme.px(20)
                anchors.topMargin: Theme.px(4)
                spacing: Theme.px(12)
                visible: win.step === "look"

                ListView {
                    id: avatarRow
                    Layout.fillWidth: true
                    Layout.preferredHeight: Theme.px(64)
                    orientation: ListView.Horizontal
                    spacing: Theme.px(10)
                    clip: true
                    model: win.avatars.length + 1
                    currentIndex: win.look.custom ? win.avatars.length : win.avatarIndex
                    highlightRangeMode: ListView.ApplyRange
                    preferredHighlightBegin: width / 2 - Theme.px(32)
                    preferredHighlightEnd: width / 2 + Theme.px(32)
                    highlightMoveDuration: win.calm ? 0 : 300
                    Accessible.role: Accessible.List
                    Accessible.name: qsTr("Pictures")
                    delegate: Item {
                        id: tile
                        required property int index
                        readonly property bool own: index === win.avatars.length
                        readonly property var pic: own ? null : win.avatars[index]
                        readonly property bool chosen: ListView.isCurrentItem
                        width: Theme.px(56)
                        height: Theme.px(64)
                        activeFocusOnTab: true
                        Accessible.role: Accessible.Button
                        Accessible.name: own ? qsTr("Use my own picture") : pic.label
                        Accessible.onPressAction: tile.pick()
                        Keys.onReturnPressed: tile.pick()
                        Keys.onSpacePressed: tile.pick()
                        function pick() {
                            Daemon.call("sound.play", { name: "tap" })
                            if (own)
                                pictureDialog.open()
                            else
                                win.act("avatar", { value: pic.id })
                        }
                        Rectangle {
                            anchors.centerIn: disc
                            width: disc.width + Theme.px(8)
                            height: width
                            radius: width / 2
                            color: "transparent"
                            border.width: 2
                            border.color: tile.activeFocus ? Theme.notchListen : Theme.notchInk
                            visible: tile.chosen || tile.activeFocus
                            scale: disc.scale
                        }
                        Rectangle {
                            id: disc
                            anchors.horizontalCenter: parent.horizontalCenter
                            y: Theme.px(8)
                            width: Theme.px(46)
                            height: width
                            radius: width / 2
                            color: tile.own ? Theme.notchCard : Theme.lookTile
                            border.width: tile.own ? 1 : 0
                            border.color: Theme.notchRaised
                            scale: tileHover.hovered || tile.chosen ? 1.08 : 1
                            Behavior on scale {
                                NumberAnimation { duration: 140; easing.type: Easing.OutBack }
                            }
                            Image {
                                anchors.fill: parent
                                visible: !tile.own
                                source: tile.own ? "" : win.avatarUrl(tile.pic.id)
                                sourceSize: Qt.size(Theme.px(104), Theme.px(104))
                                asynchronous: true
                                smooth: true
                                mipmap: true
                                Accessible.ignored: true
                            }
                            Icon {
                                anchors.centerIn: parent
                                visible: tile.own
                                name: "plus"
                                size: Theme.px(22)
                                color: Theme.notchInk
                            }
                            HoverHandler {
                                id: tileHover
                                cursorShape: Qt.PointingHandCursor
                            }
                            TapHandler {
                                onTapped: tile.pick()
                            }
                        }
                    }
                }

                ListView {
                    id: backgroundRow
                    Layout.fillWidth: true
                    Layout.preferredHeight: Theme.px(58)
                    orientation: ListView.Horizontal
                    spacing: Theme.px(10)
                    clip: true
                    model: win.backgrounds
                    currentIndex: win.backgroundIndex
                    highlightRangeMode: ListView.ApplyRange
                    preferredHighlightBegin: width / 2 - Theme.px(48)
                    preferredHighlightEnd: width / 2 + Theme.px(48)
                    highlightMoveDuration: win.calm ? 0 : 300
                    Accessible.role: Accessible.List
                    Accessible.name: qsTr("Backgrounds")
                    delegate: Rectangle {
                        id: thumb
                        required property var modelData
                        required property int index
                        readonly property bool chosen: ListView.isCurrentItem
                        width: Theme.px(96)
                        height: Theme.px(54)
                        y: Theme.px(2)
                        radius: Theme.px(10)
                        color: Theme.notchCard
                        border.width: chosen || activeFocus ? 3 : 0
                        border.color: Theme.notchInk
                        activeFocusOnTab: true
                        Accessible.role: Accessible.Button
                        Accessible.name: qsTr("%1 background").arg(modelData.label)
                        Accessible.onPressAction: thumb.pick()
                        Keys.onReturnPressed: thumb.pick()
                        Keys.onSpacePressed: thumb.pick()
                        function pick() {
                            Daemon.call("sound.play", { name: "tap" })
                            win.act("bg", { value: modelData.id })
                        }
                        Image {
                            anchors.fill: parent
                            anchors.margins: 3
                            source: win.backgroundUrl(thumb.modelData.id, true)
                            fillMode: Image.PreserveAspectCrop
                            asynchronous: true
                            smooth: true
                            Accessible.ignored: true
                        }
                        HoverHandler {
                            cursorShape: Qt.PointingHandCursor
                        }
                        TapHandler {
                            onTapped: thumb.pick()
                        }
                    }
                }

                Item { Layout.fillHeight: true }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.px(12)
                    Text {
                        Layout.fillWidth: true
                        text: win.problem.length > 0 ? win.problem : qsTr("Say next, previous, or a name like the cat. Say this one when you like it.")
                        color: win.problem.length > 0 ? Theme.notchDanger : Theme.notchMuted
                        font.family: Type.sans
                        font.pixelSize: Theme.px(13)
                        wrapMode: Text.WordWrap
                        textFormat: Text.PlainText
                    }
                    Pill {
                        text: qsTr("Cancel")
                        onPressed: win.act("cancel")
                    }
                    Pill {
                        text: win.look.edit ? qsTr("Save") : qsTr("Use this look")
                        solid: true
                        onPressed: win.act("confirm")
                    }
                }
            }

            ColumnLayout {
                anchors.fill: parent
                anchors.leftMargin: Theme.px(32)
                anchors.rightMargin: Theme.px(32)
                anchors.bottomMargin: Theme.px(20)
                anchors.topMargin: Theme.px(8)
                spacing: Theme.px(12)
                visible: win.step === "name"

                Text {
                    Layout.fillWidth: true
                    text: qsTr("What should the profile be called?")
                    color: Theme.notchInk
                    font.family: Type.sans
                    font.pixelSize: Theme.px(20)
                    font.weight: Font.DemiBold
                    textFormat: Text.PlainText
                    Accessible.role: Accessible.Heading
                    Accessible.name: text
                }
                Rectangle {
                    Layout.fillWidth: true
                    Layout.preferredHeight: Theme.px(52)
                    radius: Theme.px(14)
                    color: Theme.notchCard
                    border.width: input.activeFocus ? 2 : 1
                    border.color: input.activeFocus ? Theme.notchInk : Theme.notchRaised
                    Item {
                        anchors.fill: parent
                        anchors.leftMargin: Theme.px(16)
                        anchors.rightMargin: Theme.px(16)
                        TextInput {
                            id: input
                            anchors.fill: parent
                            verticalAlignment: TextInput.AlignVCenter
                            color: Theme.notchInk
                            selectionColor: Theme.notchListenDeep
                            selectedTextColor: Theme.notchInk
                            font.family: Type.sans
                            font.pixelSize: Theme.px(18)
                            maximumLength: 60
                            clip: true
                            Accessible.role: Accessible.EditableText
                            Accessible.name: qsTr("Profile name")
                            onTextEdited: typing.restart()
                            Keys.onReturnPressed: win.submitName()
                            Keys.onEnterPressed: win.submitName()
                            Keys.onEscapePressed: win.act("back")
                            Text {
                                anchors.verticalCenter: parent.verticalCenter
                                visible: input.text.length === 0
                                text: qsTr("Type a name, like Anoop or Work")
                                color: Theme.notchFaint
                                font: input.font
                                Accessible.ignored: true
                            }
                        }
                        Timer {
                            id: typing
                            interval: 250
                            onTriggered: win.act("name", { value: input.text, typed: true })
                        }
                        Connections {
                            target: win
                            function onLookChanged() {
                                if (!input.activeFocus && (win.look.name || "") !== input.text)
                                    input.text = win.look.name || ""
                            }
                        }
                    }
                }
                Text {
                    Layout.fillWidth: true
                    visible: text.length > 0
                    text: {
                        if (win.problem.length > 0)
                            return win.problem
                        const n = (win.look.name || "").trim(), f = win.look.finalName || ""
                        if (n.length === 0 || f.length === 0)
                            return ""
                        if (f.toLowerCase() !== n.toLowerCase())
                            return qsTr("You already have a profile called %1, so this one will be called %2.").arg(n).arg(f)
                        return qsTr("It will be called %1.").arg(f)
                    }
                    color: win.problem.length > 0 ? Theme.notchDanger : Theme.notchMuted
                    font.family: Type.sans
                    font.pixelSize: Theme.px(14)
                    wrapMode: Text.WordWrap
                    textFormat: Text.PlainText
                }
                Item { Layout.fillHeight: true }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.px(12)
                    Item { Layout.fillWidth: true }
                    Pill {
                        text: qsTr("Back")
                        onPressed: win.act("back")
                    }
                    Pill {
                        text: qsTr("Create profile")
                        solid: true
                        enabled: input.text.trim().length > 0
                        onPressed: win.submitName()
                    }
                }
            }

            ColumnLayout {
                anchors.centerIn: parent
                visible: win.step === "making" || win.step === "save"
                spacing: Theme.px(14)
                Rectangle {
                    Layout.alignment: Qt.AlignHCenter
                    width: Theme.px(36)
                    height: width
                    radius: width / 2
                    color: "transparent"
                    border.width: 3
                    border.color: Theme.notchRaised
                    Rectangle {
                        width: Theme.px(10)
                        height: width
                        radius: width / 2
                        color: Theme.notchInk
                        x: parent.width / 2 - width / 2
                        y: -width / 2 + 1
                        transformOrigin: Item.Center
                    }
                    RotationAnimation on rotation {
                        running: !win.calm && (win.step === "making" || win.step === "save")
                        from: 0
                        to: 360
                        duration: 1100
                        loops: Animation.Infinite
                    }
                }
                Text {
                    Layout.alignment: Qt.AlignHCenter
                    text: qsTr("Setting up %1 on your new profile…").arg(win.look.display || "")
                    color: Theme.notchMuted
                    font.family: Type.sans
                    font.pixelSize: Theme.px(15)
                    textFormat: Text.PlainText
                }
            }

            ColumnLayout {
                anchors.fill: parent
                anchors.leftMargin: Theme.px(28)
                anchors.rightMargin: Theme.px(28)
                anchors.bottomMargin: Theme.px(20)
                anchors.topMargin: Theme.px(4)
                spacing: Theme.px(10)
                visible: win.step === "import"

                Text {
                    Layout.fillWidth: true
                    text: win.firefox ? qsTr("Bring things over from one of your Firefox profiles?") : qsTr("Bring things over from your old %1 profiles?").arg(win.look.display || "")
                    color: Theme.notchInk
                    font.family: Type.sans
                    font.pixelSize: Theme.px(18)
                    font.weight: Font.DemiBold
                    wrapMode: Text.WordWrap
                    textFormat: Text.PlainText
                    Accessible.role: Accessible.Heading
                    Accessible.name: text
                }
                Text {
                    Layout.fillWidth: true
                    text: qsTr("Bookmarks, history, passwords, extensions and settings are copied. Your old profiles stay as they are. Some sites may ask you to sign in again.")
                    color: Theme.notchMuted
                    font.family: Type.sans
                    font.pixelSize: Theme.px(13)
                    wrapMode: Text.WordWrap
                    textFormat: Text.PlainText
                }
                ListView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    spacing: Theme.px(6)
                    model: win.profiles
                    Accessible.role: Accessible.List
                    Accessible.name: qsTr("Old profiles")
                    delegate: Rectangle {
                        id: row
                        required property var modelData
                        readonly property bool on: win.picked.indexOf(modelData.dir) >= 0
                        readonly property bool main: win.mainDir === modelData.dir
                        width: ListView.view.width
                        height: Theme.px(56)
                        radius: Theme.px(12)
                        color: rowHover.hovered ? Theme.notchRaised : Theme.notchCard
                        border.width: activeFocus ? 2 : 0
                        border.color: Theme.notchInk
                        activeFocusOnTab: true
                        Accessible.role: win.firefox ? Accessible.RadioButton : Accessible.CheckBox
                        Accessible.name: modelData.name
                        Accessible.checkable: true
                        Accessible.checked: on
                        Accessible.onToggleAction: win.toggle(modelData.dir)
                        Accessible.onPressAction: win.toggle(modelData.dir)
                        Keys.onSpacePressed: win.toggle(modelData.dir)
                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: Theme.px(14)
                            anchors.rightMargin: Theme.px(14)
                            spacing: Theme.px(12)
                            Rectangle {
                                width: Theme.px(22)
                                height: width
                                radius: win.firefox ? width / 2 : Theme.px(6)
                                color: row.on ? Theme.notchInk : "transparent"
                                border.width: 2
                                border.color: Theme.notchInk
                                Icon {
                                    anchors.centerIn: parent
                                    visible: row.on
                                    name: "check"
                                    size: Theme.px(16)
                                    color: Theme.notchBg
                                }
                            }
                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 0
                                Text {
                                    Layout.fillWidth: true
                                    text: row.modelData.name + (row.modelData.current ? qsTr("  ·  in use") : "")
                                    color: Theme.notchInk
                                    font.family: Type.sans
                                    font.pixelSize: Theme.px(15)
                                    font.weight: Font.DemiBold
                                    elide: Text.ElideRight
                                    textFormat: Text.PlainText
                                }
                                Text {
                                    Layout.fillWidth: true
                                    text: [row.modelData.email || "", row.modelData.bytes === null || row.modelData.bytes === undefined ? qsTr("measuring…") : Platform.formatBytes(row.modelData.bytes)].filter(s => s.length > 0).join("  ·  ")
                                    color: Theme.notchMuted
                                    font.family: Type.sans
                                    font.pixelSize: Theme.px(12)
                                    elide: Text.ElideRight
                                    textFormat: Text.PlainText
                                }
                            }
                            Rectangle {
                                visible: !win.firefox && row.on && win.picked.length > 1
                                implicitWidth: mainLabel.implicitWidth + Theme.px(20)
                                implicitHeight: Theme.px(28)
                                radius: height / 2
                                color: row.main ? Theme.notchInk : "transparent"
                                border.width: 1
                                border.color: Theme.notchInk
                                Accessible.role: Accessible.Button
                                Accessible.name: qsTr("Make %1 the main profile").arg(row.modelData.name)
                                Accessible.onPressAction: win.mainDir = row.modelData.dir
                                Text {
                                    id: mainLabel
                                    anchors.centerIn: parent
                                    text: row.main ? qsTr("Main") : qsTr("Make main")
                                    color: row.main ? Theme.notchBg : Theme.notchInk
                                    font.family: Type.sans
                                    font.pixelSize: Theme.px(12)
                                    font.weight: Font.DemiBold
                                    Accessible.ignored: true
                                }
                                TapHandler {
                                    onTapped: win.mainDir = row.modelData.dir
                                }
                            }
                        }
                        HoverHandler {
                            id: rowHover
                            cursorShape: Qt.PointingHandCursor
                        }
                        TapHandler {
                            onTapped: win.toggle(row.modelData.dir)
                        }
                    }
                }
                Text {
                    Layout.fillWidth: true
                    visible: !win.firefox && win.picked.length > 1
                    text: qsTr("The main profile fills your Setu profile. The others come in as their own profiles.")
                    color: Theme.notchMuted
                    font.family: Type.sans
                    font.pixelSize: Theme.px(12)
                    wrapMode: Text.WordWrap
                    textFormat: Text.PlainText
                }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.px(12)
                    Text {
                        Layout.fillWidth: true
                        visible: win.problem.length > 0
                        text: win.problem
                        color: Theme.notchDanger
                        font.family: Type.sans
                        font.pixelSize: Theme.px(13)
                        wrapMode: Text.WordWrap
                        textFormat: Text.PlainText
                    }
                    Item { Layout.fillWidth: true; visible: win.problem.length === 0 }
                    Pill {
                        text: qsTr("Start empty")
                        onPressed: win.act("import", { dirs: [] })
                    }
                    Pill {
                        text: win.picked.length === win.profiles.length && win.picked.length > 1 ? qsTr("Bring everything") : qsTr("Bring these over")
                        solid: true
                        enabled: win.picked.length > 0
                        onPressed: win.act("import", { dirs: win.picked, main: win.mainDir })
                    }
                }
            }
        }
    }

    function submitName() {
        if (input.text.trim().length === 0)
            return
        typing.stop()
        win.problem = ""
        Daemon.call("look.act", { id: win.look.id, op: "name", value: input.text, typed: true }, (ok, r, e) => {
            if (!ok) {
                win.problem = e ? e.message : ""
                return
            }
            win.act("create")
        })
    }
}
