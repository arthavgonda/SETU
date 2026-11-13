import QtQuick
import QtQuick.Templates as T
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

Window {
    id: win
    readonly property string kind: "wizard"
    property string initialPage
    property int step: 1
    property bool forward: true
    signal finished

    readonly property var steps: [
        { key: "welcome", file: "wizard/StepWelcome.qml" },
        { key: "microphone", file: "wizard/StepMicrophone.qml" },
        { key: "models", file: "wizard/StepModels.qml" },
        { key: "setup", file: "wizard/StepSetup.qml" },
        { key: "voice", file: "wizard/StepVoice.qml" },
        { key: "browser", file: "wizard/StepBrowser.qml" },
        { key: "preferences", file: "wizard/StepPreferences.qml" },
        { key: "done", file: "wizard/StepDone.qml" }
    ]
    readonly property int last: steps.length
    readonly property var page: stack.currentItem
    readonly property bool explains: Settings.values["ui.readAloud"] !== false
    property bool finishing: false
    property string setuLine: ""
    property string heardLine: ""
    property bool thinking: false
    property bool listening: false

    width: 960
    height: 680
    minimumWidth: 720
    minimumHeight: 560
    visible: true
    color: Theme.bgBase
    title: qsTr("Set up %1").arg(Brand.productName)

    property bool timedFirst: false
    property real switchStarted: 0
    onFrameSwapped: {
        if (!Platform.overrides.timing)
            return
        if (!timedFirst) {
            timedFirst = true
            console.warn("setu-timing first-frame-ms", Platform.processMs())
        }
        if (switchStarted > 0) {
            console.warn("setu-timing page-switch-ms", Date.now() - switchStarted)
            switchStarted = 0
        }
    }

    Component.onCompleted: {
        const saved = Number(Settings.values["setup.step"] ?? 1)
        step = Math.max(1, Math.min(last, isNaN(saved) || saved > last ? 1 : saved))
        stack.replace(null, Qt.resolvedUrl(steps[step - 1].file), {}, T.StackView.Immediate)
    }

    Component.onDestruction: {
        if (Daemon.connected) {
            Daemon.call("onboarding.listen", { choices: [] })
            Daemon.call("onboarding.converse", { screen: null })
        }
    }

    function go(n) {
        if (n < 1 || n > last || n === step || finishing)
            return
        if (page)
            page.leaving()
        forward = n > step
        step = n
        heardLine = ""
        setuLine = ""
        switchStarted = Date.now()
        stack.replace(null, Qt.resolvedUrl(steps[n - 1].file))
        if (n === last)
            Settings.patch({ "setup.completed": true, "setup.step": n })
        else if (Settings.values["setup.step"] !== n)
            Settings.set("setup.step", n)
    }
    function next() {
        if (!page || page.busy && !page.canContinue)
            return
        if (!page.canContinue)
            return
        if (page.commit() === false)
            return
        if (step === last) {
            finish()
            return
        }
        go(step + 1)
    }
    function back() {
        if (step > 1 && page && page.showBack)
            go(step - 1)
    }
    function skip() {
        if (page && page.skippable)
            go(step + 1)
    }
    function finish() {
        finishing = true
        Daemon.call("onboarding.converse", { screen: null })
        Settings.patch({ "setup.completed": true, "setup.step": win.last })
        finishTimer.start()
    }
    function screen() {
        if (!page)
            return null
        return {
            step: page.key,
            title: page.title,
            summary: page.summary,
            say: page.spoken.length > 0 ? page.spoken : undefined,
            options: (page.voiceChoices || []).map(c => ({ id: c.id, label: c.label, hint: c.hint || "", phrases: c.phrases || [] })),
            state: page.screenState
        }
    }
    function sync(explain) {
        if (!page || !Daemon.connected || finishing)
            return
        Daemon.call("onboarding.listen", { choices: (page.voiceChoices || []).map(c => ({ id: c.id, phrases: c.phrases || [c.label] })) })
        Daemon.call("onboarding.converse", { screen: screen(), explain: explain && explains }, (ok, r) => {
            if (ok)
                win.listening = r.listening === true
        })
    }
    function act(action) {
        if (!page)
            return
        switch (action) {
        case "next": win.next(); break
        case "back": win.back(); break
        case "skip": win.skip(); break
        case "repeat": win.sync(true); break
        default: page.voiceChoice(action)
        }
    }

    Timer {
        id: finishTimer
        interval: Motion.reduced ? 600 : 1800
        onTriggered: {
            Launcher.startShell()
            win.finished()
        }
    }
    Timer {
        id: syncLater
        interval: 250
        onTriggered: win.sync(false)
    }

    Subscription {
        topics: ["wizard.command", "onboarding.answer", "onboarding.intent", "onboarding.heard", "onboarding.say", "onboarding.thinking", "session.state"]
        active: Daemon.connected && !win.finishing
        onReceived: (topic, d) => {
            switch (topic) {
            case "onboarding.intent":
                win.act(d.action)
                return
            case "onboarding.heard":
                win.heardLine = d.text
                return
            case "onboarding.say":
                win.setuLine = d.text
                return
            case "onboarding.thinking":
                win.thinking = d.busy === true
                return
            case "session.state":
                win.listening = d.conversing === true
                return
            case "onboarding.answer": {
                const c = win.page ? (win.page.voiceChoices || []).find(x => x.id === d.id) : null
                if (c) {
                    win.heardLine = c.label
                    win.page.voiceChoice(d.id)
                }
                return
            }
            }
            win.act(d.command)
        }
    }
    Connections {
        target: Daemon
        function onReconnected() { win.sync(false) }
    }
    Shortcut {
        sequences: ["Alt+Left"]
        onActivated: win.back()
    }

    Rectangle {
        id: progressTrack
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        height: 3
        color: Theme.bgSunken
        Accessible.role: Accessible.ProgressBar
        Accessible.name: qsTr("Step %1 of %2").arg(win.step).arg(win.last)
        Rectangle {
            height: parent.height
            width: parent.width * win.step / win.last
            color: Theme.accent
            radius: 1.5
            Behavior on width {
                NumberAnimation { duration: Motion.morph; easing.type: Easing.OutBack; easing.overshoot: Motion.overshoot * 0.5 }
            }
        }
    }

    FocusScope {
        id: pane
        anchors.fill: parent
        anchors.topMargin: progressTrack.height
        focus: true
        LayoutMirroring.enabled: Platform.rightToLeft
        LayoutMirroring.childrenInherit: true
        Keys.onReturnPressed: win.next()
        Keys.onEnterPressed: win.next()

        Banner {
            id: banner
            anchors.top: parent.top
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.margins: Theme.s4
            visible: !Daemon.connected
            level: Daemon.status === "stopped" ? "danger" : "warning"
            text: Daemon.status === "stopped" ? qsTr("%1 isn't running.").arg(Brand.productName) : qsTr("Reconnecting…")
            actionText: Daemon.status === "stopped" ? qsTr("Start") : ""
            dismissible: false
            onAction: Daemon.startDaemon()
        }

        Flickable {
            id: scroller
            anchors.top: banner.visible ? banner.bottom : parent.top
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: footer.top
            contentWidth: width
            contentHeight: Math.max(height, stack.implicitHeight + Theme.s7 * 2)
            boundsBehavior: Flickable.StopAtBounds
            clip: true
            T.ScrollBar.vertical: ScrollBar {}

            T.StackView {
                id: stack
                width: Math.min(scroller.width - Theme.s6 * 2, Theme.px(680))
                x: (scroller.width - width) / 2
                y: Math.max(Theme.s6, (scroller.height - implicitHeight) / 2 - Theme.s4)
                implicitHeight: currentItem ? currentItem.implicitHeight : 0
                height: implicitHeight
                Behavior on y {
                    enabled: !Motion.reduced
                    NumberAnimation { duration: Motion.morph; easing.type: Easing.OutBack; easing.overshoot: Motion.overshoot * 0.6 }
                }
                onCurrentItemChanged: {
                    const p = currentItem
                    if (!p)
                        return
                    p.width = Qt.binding(() => stack.width)
                    Qt.callLater(() => {
                        win.sync(true)
                        const first = p.nextItemInFocusChain(true)
                        scroller.contentY = 0
                        if (first)
                            first.forceActiveFocus()
                    })
                }
                replaceEnter: Transition {
                    ParallelAnimation {
                        NumberAnimation { property: "opacity"; from: 0; to: 1; duration: Motion.spring; easing.type: Easing.OutCubic }
                        NumberAnimation { property: "x"; from: Motion.reduced ? 0 : (win.forward ? 1 : -1) * Theme.px(70); to: 0; duration: Motion.morph; easing.type: Easing.OutBack; easing.overshoot: Motion.overshoot }
                        NumberAnimation { property: "scale"; from: Motion.reduced ? 1 : 0.94; to: 1; duration: Motion.morph; easing.type: Easing.OutBack; easing.overshoot: Motion.overshoot }
                    }
                }
                replaceExit: Transition {
                    ParallelAnimation {
                        NumberAnimation { property: "opacity"; from: 1; to: 0; duration: Motion.fade; easing.type: Easing.InCubic }
                        NumberAnimation { property: "x"; from: 0; to: Motion.reduced ? 0 : (win.forward ? -1 : 1) * Theme.px(50); duration: Motion.fade; easing.type: Easing.InCubic }
                        NumberAnimation { property: "scale"; from: 1; to: Motion.reduced ? 1 : 1.03; duration: Motion.fade; easing.type: Easing.InCubic }
                    }
                }
            }
        }

        Connections {
            target: win.page
            ignoreUnknownSignals: true
            function onToast(text) { toasts.show(text) }
            function onAdvance() { win.next() }
            function onRetreat() { win.back() }
            function onSay(text) {
                win.setuLine = text
                Daemon.call("onboarding.listen", { say: text })
            }
            function onVoiceChoicesChanged() { syncLater.restart() }
            function onScreenStateChanged() { syncLater.restart() }
        }

        Rectangle {
            id: footer
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: footerRow.implicitHeight + Theme.s4 * 2
            color: Theme.bgBase
            RowLayout {
                id: footerRow
                anchors.fill: parent
                anchors.leftMargin: Theme.s5
                anchors.rightMargin: Theme.s5
                spacing: Theme.s3
                IconButton {
                    label: qsTr("Back")
                    iconName: "arrow-left"
                    enabled: win.step > 1 && win.page && win.page.showBack
                    opacity: enabled ? 1 : 0
                    onClicked: win.back()
                }
                VoiceStrip {
                    Layout.fillWidth: true
                    listening: win.listening
                    thinking: win.thinking
                    setuText: win.setuLine
                    heardText: win.heardLine
                }
                Button {
                    visible: win.page && win.page.skippable
                    variant: "quiet"
                    text: win.page ? win.page.skipText : ""
                    onClicked: win.skip()
                }
                Button {
                    visible: win.page ? win.page.showPrimary : false
                    variant: "primary"
                    text: win.page ? win.page.primaryText : ""
                    iconName: win.page ? win.page.primaryIcon : ""
                    enabled: win.page && win.page.canContinue
                    busy: win.page && win.page.busy
                    onClicked: win.next()
                }
            }
        }
    }

    Rectangle {
        anchors.fill: parent
        visible: opacity > 0
        opacity: win.finishing ? 1 : 0
        color: Theme.bgBase
        z: 200
        Behavior on opacity {
            NumberAnimation { duration: Motion.spring }
        }
        Accessible.role: Accessible.Pane
        Accessible.name: qsTr("Opening %1").arg(Brand.productName)
        MouseArea {
            anchors.fill: parent
            enabled: win.finishing
            acceptedButtons: Qt.AllButtons
        }
        ColumnLayout {
            anchors.centerIn: parent
            spacing: Theme.s3
            LottieAnimation {
                Layout.alignment: Qt.AlignHCenter
                Layout.preferredWidth: Theme.px(220)
                Layout.preferredHeight: Theme.px(220)
                source: win.finishing ? "qrc:/lottie/setting-up.json" : ""
                playing: !Motion.reduced
                stillProgress: 0.3
                Accessible.ignored: true
            }
            Label {
                Layout.alignment: Qt.AlignHCenter
                text: qsTr("Opening %1").arg(Brand.productName)
                role: "title"
            }
        }
    }

    ToastHost {
        id: toasts
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.margins: Theme.s5
        anchors.bottomMargin: Theme.px(96)
        z: 100
    }
    PromptHost {
        onToast: text => toasts.show(text)
    }
}
