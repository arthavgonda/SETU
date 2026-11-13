import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Setu.ShellHost
import Ledger

Window {
    id: win
    property string phase: "hidden"
    property int interaction: 0
    property string partial
    property string finalText
    property var action: ({})
    property var progress: null
    property var ask: null
    property var confirm: null
    property var reply: null
    property int replyLine: 0
    property string doneText
    property string problemText
    property var problemAction: null
    property var local: null
    property int scanIndex: -1
    property var board: ({ timers: [], stopwatch: null, todos: { items: [], doneToday: 0, total: 0 }, notes: [], reminders: [] })
    property var hud: null
    property var peek: null
    property var media: null
    property bool trackPeek: false
    property bool hovering: false
    property bool pinned: false
    property bool rejected: false
    property real now: Date.now()
    property real skew: 0
    signal quitConfirmed

    readonly property var session: Daemon.session
    readonly property bool muted: Daemon.mic.muted === true
    readonly property bool awake: ["capturing", "grace", "active"].includes(session.state) || session.conversing === true
    readonly property bool coding: session.mode === "coding"
    readonly property bool dictating: session.mode === "dictation"
    readonly property string notchMode: Settings.values["ui.notch"] ?? "always"
    readonly property bool scanning: Settings.values["ui.switchScanning"] === true
    readonly property string stopWord: Settings.values["voice.stopWord"] ?? "stop"
    readonly property bool calm: Motion.reduced

    readonly property var timers: board.timers || []
    readonly property var focusTimer: timers.find(t => t.kind === "focus" || t.kind === "break") || null
    readonly property var countdown: timers.filter(t => t.kind === "timer").sort((a, b) => a.endsAt - b.endsAt)[0] || null
    property var jobs: []
    readonly property var runningJobs: jobs.filter(j => ["queued", "download", "verify", "install", "waiting"].includes(j.step))
    function jobStep(j) {
        if (!j)
            return ""
        if (j.paused)
            return qsTr("paused")
        switch (j.step) {
        case "download": return j.percent !== null && j.percent !== undefined ? qsTr("%1%").arg(Math.round(j.percent)) : qsTr("downloading")
        case "verify": return qsTr("verifying")
        case "waiting": return qsTr("waiting for you")
        case "install": return j.percent !== null && j.percent !== undefined ? qsTr("installing %1%").arg(Math.round(j.percent)) : qsTr("installing")
        default: return qsTr("starting")
        }
    }
    readonly property var watch: board.stopwatch || null
    readonly property bool ticking: focusTimer !== null || countdown !== null || (watch !== null && watch.running)
    readonly property var todos: board.todos || ({ items: [], doneToday: 0, total: 0 })
    readonly property var reminder: (board.reminders || [])[0] || null
    readonly property bool musicOn: media !== null && media.playing === true
    property var sounds: []
    property bool soundsOpen: false
    readonly property real mediaPos: {
        if (!media || media.positionMs === null || media.positionMs === undefined)
            return -1
        const moved = media.playing ? Math.max(0, now + skew - media.at) * (media.rate || 1) : 0
        const p = media.positionMs + moved
        return media.durationMs ? Math.min(p, media.durationMs) : p
    }
    readonly property int lyricIndex: {
        const l = media && media.lyrics ? media.lyrics : null
        if (!l || l.length === 0 || mediaPos < 0)
            return -1
        let k = -1
        for (let i = 0; i < l.length; ++i) {
            if (l[i][0] <= mediaPos + 250)
                k = i
            else
                break
        }
        return k
    }

    readonly property string view: {
        if (local)
            return "confirm"
        switch (phase) {
        case "asking": return ask ? "ask" : "think"
        case "confirming": return confirm ? "confirm" : "think"
        case "problem": return "problem"
        case "listening": return "listen"
        case "thinking": return "think"
        case "acting": return "act"
        case "speaking": return "speak"
        case "done": return hud ? "hud" : "done"
        }
        if (hud)
            return "hud"
        if (peek)
            return "peek"
        if (soundsOpen && sounds.length > 0)
            return "sounds"
        if (hovering || pinned)
            return "home"
        if (trackPeek && media)
            return "track"
        if (focusTimer)
            return "focusMini"
        if (countdown)
            return "timerMini"
        if (watch)
            return "watchMini"
        if (runningJobs.length > 0)
            return "jobsMini"
        if (musicOn)
            return "musicMini"
        return "idle"
    }
    readonly property bool expanded: ["ask", "confirm", "problem", "home", "peek", "sounds"].includes(view)
    readonly property string mood: {
        if (rejected)
            return "sad"
        switch (view) {
        case "listen": return "listen"
        case "think": return "think"
        case "act": return "happy"
        case "speak": return "talk"
        case "done": return "happy"
        case "hud": return "idle"
        case "ask": return "idle"
        case "confirm": return "idle"
        case "problem": return "sad"
        case "peek": return peek && ["timer", "focus", "break", "reminder"].includes(peek.kind) ? "wow" : (peek && peek.kind === "install" && !peek.ok ? "sad" : "happy")
        case "focusMini": return focusTimer && focusTimer.kind === "focus" ? "focus" : "happy"
        case "watchMini": return "happy"
        case "timerMini": return "idle"
        case "jobsMini": return "focus"
        case "musicMini": return "happy"
        case "track": return "happy"
        }
        return awake || hovering || pinned ? "idle" : "sleepy"
    }
    readonly property bool shown: {
        if (notchMode === "off")
            return ["ask", "confirm", "problem"].includes(view)
        if (notchMode === "active")
            return view !== "idle" || awake
        return true
    }

    readonly property int pad: Theme.px(14)
    readonly property int compactH: Theme.px(32)
    readonly property int barH: Theme.px(48)
    readonly property int wide: Theme.px(560)
    readonly property int shoulder: Theme.px(10)
    readonly property int edge: Theme.px(24)
    readonly property size target: {
        const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v))
        switch (view) {
        case "idle": return Qt.size(Theme.px(muted || coding || dictating ? 168 : 132), compactH)
        case "timerMini":
        case "focusMini": return Qt.size(Theme.px(206), compactH)
        case "watchMini": return Qt.size(Theme.px(250), compactH)
        case "jobsMini": return Qt.size(Theme.px(runningJobs.length > 1 ? 236 : 262), compactH)
        case "musicMini": return Qt.size(Theme.px(196), compactH)
        case "track": return Qt.size(clamp(trackRow.implicitWidth + pad * 2, Theme.px(300), Theme.px(480)), barH)
        case "listen": return Qt.size(clamp(listenRow.implicitWidth + pad * 2, Theme.px(320), Theme.px(540)), Math.max(barH, listenRow.implicitHeight + Theme.px(18)))
        case "think": return Qt.size(clamp(thinkRow.implicitWidth + pad * 2, Theme.px(260), Theme.px(460)), barH)
        case "act": return Qt.size(clamp(actRow.implicitWidth + pad * 2, Theme.px(260), Theme.px(500)), barH)
        case "speak": return Qt.size(clamp(speakRow.implicitWidth + pad * 2, Theme.px(300), Theme.px(560)), Math.max(barH, speakRow.implicitHeight + Theme.px(18)))
        case "done": return Qt.size(clamp(doneRow.implicitWidth + pad * 2, Theme.px(220), Theme.px(520)), barH)
        case "hud": return Qt.size(Theme.px(380), barH)
        case "ask": return Qt.size(wide, askCol.implicitHeight + pad * 2)
        case "confirm": return Qt.size(wide, confirmCol.implicitHeight + pad * 2)
        case "problem": return Qt.size(Theme.px(520), problemCol.implicitHeight + pad * 2)
        case "home": return Qt.size(wide, homeCol.implicitHeight + pad * 2)
        case "sounds": return Qt.size(Theme.px(480), soundsCol.implicitHeight + pad * 2)
        case "peek": return Qt.size(Theme.px(500), peekCol.implicitHeight + pad * 2)
        }
        return Qt.size(Theme.px(132), compactH)
    }

    visible: shown
    width: wide + (shoulder + edge) * 2
    height: Theme.px(400)
    color: "transparent"
    title: qsTr("%1 notch").arg(Brand.productName)

    function place() {
        OverlayHost.placeTop(win, width, height)
        OverlayHost.raiseNotch(win)
        Qt.callLater(syncShape)
    }

    function syncShape() {
        const w = Math.max(body.width, target.width) + (shoulder + Theme.px(6)) * 2
        const h = Math.max(body.height, target.height) + Theme.px(10)
        const rects = [Qt.rect(Math.floor((win.width - w) / 2), 0, Math.ceil(w), Math.ceil(h))]
        OverlayHost.setShape(win, rects)
    }

    function show(p) {
        const was = phase
        phase = p
        if (was !== p)
            announce()
    }

    function hide() {
        phase = "hidden"
        ask = null
        confirm = null
        reply = null
        local = null
        progress = null
        scanIndex = -1
    }

    function announce() {
        const detail = Settings.values["ui.announceDetail"] ?? "normal"
        const brief = detail === "brief"
        const verbose = detail === "verbose"
        let text = ""
        switch (phase) {
        case "listening":
            text = verbose ? qsTr("Listening. Say stop to cancel.") : qsTr("Listening")
            break
        case "acting":
            text = brief ? qsTr("Working") : (action.label || "")
            break
        case "asking":
            text = ask ? ask.question : ""
            if (ask && !brief)
                text += " " + ask.options.map(o => o.n + " " + o.label).join(", ")
            break
        case "confirming":
            if (confirm)
                text = brief ? confirm.title : confirm.title + ". " + confirm.consequence + (verbose && confirm.facts.length > 0 ? " " + confirm.facts.join(". ") : "")
            else if (local)
                text = local.title
            break
        case "speaking":
            text = brief ? "" : (reply ? reply.text : "")
            break
        case "done":
            text = doneText
            break
        case "problem":
            text = problemText
            break
        }
        if (text.length > 0)
            Announcer.announce(plate, text, phase === "problem")
    }

    function stop() {
        if (local) {
            local = null
            hide()
            return
        }
        Daemon.call("action.cancel", {})
    }

    function choose(n) {
        if (ask)
            Daemon.call("prompt.answer", { id: ask.id, answer: n, via: "key" })
    }

    function askQuit() {
        local = { title: qsTr("Quit %1?").arg(Brand.productName), consequence: qsTr("Voice control stops until you start %1 again.").arg(Brand.productName), actionLabel: qsTr("Quit") }
        show("confirming")
    }

    function answerConfirm(yes, via) {
        if (local) {
            if (yes)
                win.quitConfirmed()
            local = null
            hide()
        } else if (confirm) {
            Daemon.call("prompt.answer", { id: confirm.id, answer: yes ? "yes" : "no", via: via })
        }
    }

    function clock(ms) {
        const total = Math.max(0, Math.round(ms / 1000))
        const h = Math.floor(total / 3600)
        const m = Math.floor((total % 3600) / 60)
        const s = total % 60
        const two = v => (v < 10 ? "0" : "") + v
        return h > 0 ? h + ":" + two(m) + ":" + two(s) : m + ":" + two(s)
    }

    function left(t) {
        return t ? Math.max(0, t.endsAt - (now + skew)) : 0
    }

    function done(t) {
        return t && t.totalMs > 0 ? 1 - left(t) / t.totalMs : 0
    }

    function watchMs() {
        if (!watch)
            return 0
        return watch.carriedMs + (watch.running ? Math.max(0, now + skew - watch.startedAt) : 0)
    }

    function interactiveItems() {
        let list = []
        const add = it => {
            if (it && it.visible)
                list.push(it)
        }
        if (view === "ask")
            for (let i = 0; i < optionRepeater.count; ++i)
                add(optionRepeater.itemAt(i))
        if (view === "confirm") {
            add(noButton)
            add(yesButton)
        }
        if (view === "problem") {
            add(fixButton)
            add(dismissButton)
        }
        return list
    }

    function loadBoard() {
        Daemon.call("notch.state", {}, (ok, r) => {
            if (ok) {
                win.skew = r.now ? r.now - Date.now() : 0
                win.board = r
            }
        })
        Daemon.call("media.now", {}, (ok, r) => {
            if (ok)
                win.setMedia(r)
        })
        Daemon.call("jobs.state", {}, (ok, r) => {
            if (ok)
                win.jobs = r.jobs || []
        })
    }

    function setSounds(d) {
        sounds = d && d.list ? d.list : []
        soundsOpen = !!(d && d.listOpen)
    }

    function setMedia(d) {
        setSounds(d)
        const before = media
        const next = d && !d.none ? d : null
        media = next
        if (next && next.playing && (!before || before.title !== next.title || before.key !== next.key)) {
            trackPeek = true
            trackTimer.restart()
        }
        if (!next)
            trackPeek = false
    }

    function control(action) {
        if (!media)
            return
        Daemon.call("sound.play", { name: "tap" })
        Daemon.call("media.control", { key: media.key, action: action })
    }

    onViewChanged: {
        syncShape()
        if (OverlayHost.shots.length > 0 && Platform.overrides.preview !== "notch")
            shotTimer.restart()
        if (view === "listen" && !calm)
            plate.bump()
    }
    onTargetChanged: syncShape()
    onShownChanged: if (shown) Qt.callLater(place)

    Component.onCompleted: {
        OverlayHost.prepare(win, false)
        place()
        if (Daemon.connected)
            loadBoard()
    }

    Connections {
        target: OverlayHost
        function onScreensChanged() {
            win.place()
        }
    }

    Connections {
        target: Daemon
        function onReconnected() {
            win.loadBoard()
        }
    }

    Timer {
        id: shotTimer
        interval: 700
        onTriggered: OverlayHost.capture(win, OverlayHost.shots + "/notch-" + win.view + ".png")
    }

    Timer {
        interval: 250
        repeat: true
        running: win.shown && (win.ticking || win.view === "home" || win.view === "track" || (win.musicOn && win.media.lyrics))
        triggeredOnStart: true
        onTriggered: win.now = Date.now()
    }

    Timer {
        id: doneTimer
        interval: 1700
        onTriggered: if (win.phase === "done") win.hide()
    }
    Timer {
        id: problemTimer
        interval: 9000 * Math.max(1, Motion.timeoutScale || 1)
        onTriggered: if (win.phase === "problem") win.hide()
    }
    Timer {
        id: hudTimer
        interval: 1900
        onTriggered: win.hud = null
    }
    Timer {
        id: peekTimer
        interval: 7000
        onTriggered: win.peek = null
    }
    Timer {
        id: trackTimer
        interval: 4200
        onTriggered: win.trackPeek = false
    }
    Timer {
        id: rejectTimer
        interval: 1400
        onTriggered: win.rejected = false
    }
    Timer {
        id: hoverIn
        interval: 220
        onTriggered: win.hovering = true
    }
    Timer {
        id: hoverOut
        interval: 480
        onTriggered: {
            win.hovering = false
            if (win.pinned)
                pinTimer.restart()
        }
    }
    Timer {
        id: pinTimer
        interval: 6000
        onTriggered: win.pinned = false
    }

    Timer {
        interval: 1500 * Math.max(1, Motion.timeoutScale || 1)
        repeat: true
        running: win.shown && win.scanning && win.interactiveItems().length > 0
        onTriggered: win.scanIndex = (win.scanIndex + 1) % Math.max(1, win.interactiveItems().length)
    }

    Hotkeys {
        id: hotkeys
        active: win.shown && hotkeys.available && win.view !== "idle" && win.view !== "track" && !win.view.endsWith("Mini")
        keys: {
            let k = ["Escape"]
            if (win.view === "ask" && win.ask)
                for (let i = 1; i <= Math.min(9, win.ask.options.length); ++i)
                    k.push(String(i))
            if (win.view === "confirm" || win.scanning)
                k.push("Return")
            return k
        }
        onPressed: key => {
            if (key === "Escape") {
                if (win.view === "confirm")
                    win.answerConfirm(false, "key")
                else if (win.view === "problem" || win.view === "done" || win.view === "peek" || win.view === "home") {
                    win.peek = null
                    win.pinned = false
                    win.hide()
                } else
                    win.stop()
            } else if (key === "Return") {
                if (win.scanning && win.scanIndex >= 0) {
                    const items = win.interactiveItems()
                    const it = items[win.scanIndex % items.length]
                    if (it && it.clicked)
                        it.clicked()
                } else if (win.view === "confirm") {
                    win.answerConfirm(true, "key")
                }
            } else {
                win.choose(Number(key))
            }
        }
    }

    Subscription {
        topics: ["notch.state", "notch.alert", "speaker.rejected", "media.now", "job.state"]
        onReceived: (topic, d) => {
            if (topic === "job.state") {
                win.jobs = d.jobs || []
            } else if (topic === "media.now") {
                win.setMedia(d)
            } else if (topic === "notch.state") {
                win.skew = d.now ? d.now - Date.now() : win.skew
                win.board = d
            } else if (topic === "notch.alert") {
                win.peek = d
                peekTimer.interval = d.kind === "notes" || d.kind === "todos" ? 6500 : 9000
                peekTimer.restart()
            } else if (topic === "speaker.rejected") {
                win.rejected = true
                rejectTimer.restart()
            }
        }
    }

    Subscription {
        topics: ["wake.detected", "transcript.partial", "transcript.final", "action.started", "action.progress", "action.finished", "prompt.ask", "prompt.update", "prompt.confirm", "prompt.closed", "reply.speak", "reply.line", "reply.done", "notice.problem", "interaction.ended"]
        onReceived: (topic, d) => {
            if (d.interaction !== undefined && d.interaction !== null && d.interaction < win.interaction && topic !== "reply.line" && topic !== "reply.done")
                return
            switch (topic) {
            case "wake.detected":
                win.interaction = d.interaction
                win.partial = ""
                win.finalText = ""
                win.action = ({})
                win.reply = null
                win.ask = null
                win.confirm = null
                win.peek = null
                win.pinned = false
                win.show("listening")
                break
            case "transcript.partial":
                win.interaction = d.interaction
                win.partial = d.text
                if (win.phase === "hidden" || win.phase === "done")
                    win.show("listening")
                break
            case "transcript.final":
                win.interaction = d.interaction
                win.finalText = d.text
                if (win.phase !== "asking" && win.phase !== "confirming")
                    win.show("thinking")
                break
            case "action.started":
                win.action = d
                win.progress = null
                win.show("acting")
                break
            case "action.progress":
                win.progress = d
                break
            case "action.finished":
                if (d.result === "success") {
                    win.doneText = d.message
                    win.action = Object.assign({}, win.action, { undoable: d.undoable, id: d.id })
                    const verb = win.action.verb || ""
                    if (verb === "set_volume" || verb === "set_brightness") {
                        const m = String(d.message || "").match(/(\d+)\s*%/)
                        win.hud = { kind: verb === "set_volume" ? "volume" : "brightness", value: m ? Number(m[1]) : -1, muted: /muted/i.test(d.message || "") }
                        hudTimer.restart()
                    }
                    win.show("done")
                }
                break
            case "prompt.ask":
                win.ask = d
                win.show("asking")
                break
            case "prompt.update":
                if (win.ask && win.ask.id === d.id)
                    win.ask = Object.assign({}, win.ask, { focus: d.focus })
                break
            case "prompt.confirm":
                if (d.origin === "window")
                    break
                win.confirm = d
                win.local = null
                win.show("confirming")
                break
            case "prompt.closed":
                if ((win.ask && win.ask.id === d.id) || (win.confirm && win.confirm.id === d.id)) {
                    win.ask = null
                    win.confirm = null
                    if (d.outcome !== "answered") {
                        win.doneText = d.message || qsTr("Cancelled.")
                        win.show("done")
                        doneTimer.restart()
                    } else {
                        win.show("thinking")
                    }
                }
                break
            case "reply.speak":
                win.reply = d
                win.replyLine = 0
                if (!win.ask && !win.confirm)
                    win.show("speaking")
                break
            case "reply.line":
                if (win.reply && win.reply.id === d.id)
                    win.replyLine = d.index
                break
            case "reply.done":
                if (win.reply && win.reply.id === d.id && win.phase === "speaking") {
                    win.doneText = win.reply.lines.length > 0 ? win.reply.lines[win.reply.lines.length - 1] : win.reply.text
                    win.show("done")
                }
                break
            case "notice.problem":
                win.problemText = d.message
                win.problemAction = d.action
                win.show("problem")
                problemTimer.restart()
                break
            case "interaction.ended":
                if (d.outcome === "discarded" && (win.phase === "listening" || win.phase === "thinking"))
                    win.hide()
                else if (["done", "speaking", "thinking", "listening", "acting"].includes(win.phase)) {
                    if (win.phase !== "done") {
                        win.doneText = d.outcome === "cancelled" ? qsTr("Cancelled.") : win.doneText
                        win.show("done")
                    }
                    doneTimer.restart()
                }
                break
            }
        }
    }

    component Pane: Item {
        property string name
        readonly property bool active: win.view === name
        anchors.fill: parent
        anchors.leftMargin: win.pad
        anchors.rightMargin: win.pad
        anchors.topMargin: win.expanded ? win.pad : 0
        anchors.bottomMargin: win.expanded ? win.pad : 0
        opacity: active ? 1 : 0
        scale: active || win.calm ? 1 : 0.94
        visible: opacity > 0.01
        enabled: active
        Behavior on opacity {
            NumberAnimation { duration: win.calm ? 0 : 170; easing.type: Easing.OutCubic }
        }
        Behavior on scale {
            NumberAnimation { duration: win.calm ? 0 : 320; easing.type: Easing.OutBack; easing.overshoot: 1.8 }
        }
    }

    component Line: Text {
        property bool muted: false
        property int size: 15
        color: muted ? Theme.notchMuted : Theme.notchInk
        font.family: Type.sans
        font.pixelSize: Theme.px(size)
        font.weight: muted ? Font.Normal : Font.DemiBold
        textFormat: Text.PlainText
        renderType: Text.NativeRendering
        elide: Text.ElideRight
        verticalAlignment: Text.AlignVCenter
        Accessible.name: text
    }

    component Card: Rectangle {
        id: card
        default property alias content: inner.data
        property int gap: Theme.px(14)
        radius: Theme.px(18)
        color: Theme.notchCard
        implicitHeight: inner.implicitHeight + gap * 2
        implicitWidth: inner.implicitWidth + gap * 2
        RowLayout {
            id: inner
            anchors.fill: parent
            anchors.margins: card.gap
            spacing: Theme.px(16)
        }
    }

    component Face: Rectangle {
        property alias mood: face.mood
        implicitWidth: Theme.px(96)
        implicitHeight: Theme.px(52)
        radius: height / 2
        color: Theme.notchBg
        border.width: 1
        border.color: Theme.notchRaised
        Eyes {
            id: face
            anchors.centerIn: parent
            width: Theme.px(46)
            height: Theme.px(22)
        }
    }

    Item {
        id: body
        property real bw: win.target.width
        property real bh: win.target.height
        Behavior on bw {
            enabled: !win.calm
            SpringAnimation { spring: 3.6; damping: 0.3; epsilon: 0.4 }
        }
        Behavior on bh {
            enabled: !win.calm
            SpringAnimation { spring: 3.6; damping: 0.34; epsilon: 0.4 }
        }
        x: Math.round((win.width - bw) / 2)
        y: 0
        width: Math.round(bw)
        height: Math.round(bh)
        readonly property real corner: win.expanded ? Theme.px(26) : Math.min(height / 2, Theme.px(18))
        onWidthChanged: if (Math.abs(width - win.target.width) < 1) win.syncShape()

        Repeater {
            model: Theme.contrast ? 0 : 3
            Rectangle {
                required property int index
                x: -(index + 1) * 2
                y: -body.corner
                width: body.width + (index + 1) * 4
                height: body.height + body.corner + (index + 1) * 3 + 2
                radius: body.corner + index * 2
                color: Theme.notchShadow
                opacity: 0.22 - index * 0.06
            }
        }

        Canvas {
            id: leftShoulder
            x: -width + 1
            y: 0
            width: win.shoulder
            height: win.shoulder
            renderTarget: Canvas.Image
            Accessible.ignored: true
            onPaint: {
                const ctx = getContext("2d")
                ctx.reset()
                ctx.fillStyle = Theme.notchBg
                ctx.beginPath()
                ctx.moveTo(0, 0)
                ctx.lineTo(width, 0)
                ctx.lineTo(width, height)
                ctx.arc(0, height, width, 0, -Math.PI / 2, true)
                ctx.closePath()
                ctx.fill()
            }
        }
        Canvas {
            x: body.width - 1
            y: 0
            width: win.shoulder
            height: win.shoulder
            renderTarget: Canvas.Image
            Accessible.ignored: true
            transform: Scale { origin.x: win.shoulder / 2; xScale: -1 }
            onPaint: {
                const ctx = getContext("2d")
                ctx.reset()
                ctx.fillStyle = Theme.notchBg
                ctx.beginPath()
                ctx.moveTo(0, 0)
                ctx.lineTo(width, 0)
                ctx.lineTo(width, height)
                ctx.arc(0, height, width, 0, -Math.PI / 2, true)
                ctx.closePath()
                ctx.fill()
            }
        }

        Rectangle {
            id: plate
            y: -body.corner
            width: body.width
            height: body.height + body.corner
            radius: body.corner
            color: Theme.notchBg
            border.width: Theme.contrast ? 2 : 0
            border.color: Theme.notchInk
            Accessible.role: Accessible.Pane
            Accessible.name: win.title
            function bump() {
                squish.restart()
            }
            transform: Scale {
                id: squishScale
                origin.x: body.width / 2
                origin.y: body.corner
                xScale: 1
                yScale: 1
            }
            SequentialAnimation {
                id: squish
                ParallelAnimation {
                    NumberAnimation { target: squishScale; property: "xScale"; to: 1.035; duration: 110; easing.type: Easing.OutQuad }
                    NumberAnimation { target: squishScale; property: "yScale"; to: 0.95; duration: 110; easing.type: Easing.OutQuad }
                }
                ParallelAnimation {
                    NumberAnimation { target: squishScale; property: "xScale"; to: 1; duration: 420; easing.type: Easing.OutElastic; easing.amplitude: 1.2; easing.period: 0.4 }
                    NumberAnimation { target: squishScale; property: "yScale"; to: 1; duration: 420; easing.type: Easing.OutElastic; easing.amplitude: 1.2; easing.period: 0.4 }
                }
            }
        }

        HoverHandler {
            id: hover
            onHoveredChanged: {
                if (hovered) {
                    hoverOut.stop()
                    if (["idle", "timerMini", "focusMini", "watchMini", "jobsMini", "musicMini", "track"].includes(win.view))
                        hoverIn.restart()
                } else {
                    hoverIn.stop()
                    if (win.hovering)
                        hoverOut.restart()
                }
            }
        }
        TapHandler {
            enabled: ["idle", "timerMini", "focusMini", "watchMini", "jobsMini", "musicMini", "track"].includes(win.view)
            onTapped: {
                win.pinned = true
                pinTimer.restart()
                Daemon.call("sound.play", { name: "toggle" })
            }
        }

        Item {
            id: stage
            anchors.fill: parent
            clip: true

            Pane {
                name: "idle"
                RowLayout {
                    anchors.fill: parent
                    spacing: Theme.px(8)
                    Eyes {
                        Layout.preferredWidth: Theme.px(34)
                        Layout.preferredHeight: Theme.px(16)
                        mood: win.mood
                    }
                    Item { Layout.fillWidth: true }
                    Icon {
                        visible: win.muted
                        name: "mic-off"
                        color: Theme.notchMuted
                        size: Theme.px(15)
                    }
                    Line {
                        visible: win.coding && !win.muted
                        text: "</>"
                        size: 13
                        color: Theme.notchMuted
                    }
                    Line {
                        visible: win.dictating && !win.muted
                        text: qsTr("Aa")
                        size: 13
                        color: Theme.notchMuted
                    }
                }
            }

            Pane {
                name: "timerMini"
                RowLayout {
                    anchors.fill: parent
                    spacing: Theme.px(8)
                    Eyes {
                        Layout.preferredWidth: Theme.px(34)
                        Layout.preferredHeight: Theme.px(16)
                        mood: win.mood
                    }
                    Item { Layout.fillWidth: true }
                    NotchRing {
                        Layout.preferredWidth: Theme.px(16)
                        Layout.preferredHeight: Theme.px(16)
                        stroke: Theme.notchTimer
                        progress: win.done(win.countdown)
                    }
                    Line {
                        text: win.clock(win.left(win.countdown))
                        font.features: ({ "tnum": 1 })
                    }
                }
            }

            Pane {
                name: "jobsMini"
                RowLayout {
                    anchors.fill: parent
                    spacing: Theme.px(8)
                    Eyes {
                        Layout.preferredWidth: Theme.px(34)
                        Layout.preferredHeight: Theme.px(16)
                        mood: win.mood
                    }
                    Item { Layout.fillWidth: true }
                    Repeater {
                        model: win.runningJobs.slice(0, 3)
                        NotchRing {
                            id: jobRing
                            required property var modelData
                            readonly property bool known: modelData.percent !== null && modelData.percent !== undefined && modelData.step !== "verify"
                            Layout.preferredWidth: Theme.px(16)
                            Layout.preferredHeight: Theme.px(16)
                            stroke: modelData.step === "waiting" ? Theme.notchTimer : Theme.notchListen
                            progress: known ? modelData.percent / 100 : 0.28
                            rotation: 0
                            RotationAnimation on rotation {
                                running: !jobRing.known && !win.calm && win.view === "jobsMini"
                                from: 0
                                to: 360
                                duration: 1200
                                loops: Animation.Infinite
                            }
                            Accessible.role: Accessible.ProgressBar
                            Accessible.name: modelData.app + " " + win.jobStep(modelData)
                            Accessible.ignored: false
                        }
                    }
                    Line {
                        Layout.maximumWidth: Theme.px(150)
                        text: win.runningJobs.length > 1 ? qsTr("%1 jobs").arg(win.runningJobs.length) : (win.runningJobs.length === 1 ? win.runningJobs[0].app + "  " + win.jobStep(win.runningJobs[0]) : "")
                        font.features: ({ "tnum": 1 })
                    }
                }
            }

            Pane {
                name: "focusMini"
                RowLayout {
                    anchors.fill: parent
                    spacing: Theme.px(8)
                    Eyes {
                        Layout.preferredWidth: Theme.px(34)
                        Layout.preferredHeight: Theme.px(16)
                        mood: win.mood
                    }
                    Item { Layout.fillWidth: true }
                    NotchRing {
                        Layout.preferredWidth: Theme.px(16)
                        Layout.preferredHeight: Theme.px(16)
                        stroke: win.focusTimer && win.focusTimer.kind === "break" ? Theme.notchBreak : Theme.notchFocus
                        progress: win.done(win.focusTimer)
                    }
                    Line {
                        text: win.clock(win.left(win.focusTimer))
                        font.features: ({ "tnum": 1 })
                    }
                }
            }

            Pane {
                name: "watchMini"
                RowLayout {
                    anchors.fill: parent
                    spacing: Theme.px(8)
                    Eyes {
                        Layout.preferredWidth: Theme.px(34)
                        Layout.preferredHeight: Theme.px(16)
                        mood: win.mood
                    }
                    Line {
                        text: qsTr("Stopwatch")
                        color: Theme.notchTimer
                        size: 14
                    }
                    Item { Layout.fillWidth: true }
                    Line {
                        text: win.clock(win.watchMs())
                        opacity: win.watch && win.watch.running ? 1 : 0.6
                        font.features: ({ "tnum": 1 })
                    }
                }
            }

            Pane {
                name: "musicMini"
                RowLayout {
                    anchors.fill: parent
                    spacing: Theme.px(8)
                    NotchArt {
                        Layout.preferredWidth: Theme.px(22)
                        Layout.preferredHeight: Theme.px(22)
                        radius: Theme.px(5)
                        source: win.media && win.media.art ? win.media.art : ""
                    }
                    Item { Layout.fillWidth: true }
                    NotchEqualizer {
                        Layout.preferredHeight: Theme.px(16)
                        playing: win.musicOn
                    }
                }
            }

            Pane {
                name: "track"
                RowLayout {
                    id: trackRow
                    anchors.fill: parent
                    spacing: Theme.px(12)
                    NotchArt {
                        id: trackArt
                        Layout.preferredWidth: Theme.px(34)
                        Layout.preferredHeight: Theme.px(34)
                        radius: Theme.px(8)
                        source: win.media && win.media.art ? win.media.art : ""
                        scale: 1
                        Connections {
                            target: win
                            function onTrackPeekChanged() {
                                if (win.trackPeek && !win.calm)
                                    artPop.restart()
                            }
                        }
                        SequentialAnimation {
                            id: artPop
                            NumberAnimation { target: trackArt; property: "scale"; from: 0.4; to: 1.12; duration: 260; easing.type: Easing.OutBack; easing.overshoot: 2.2 }
                            NumberAnimation { target: trackArt; property: "scale"; to: 1; duration: 180; easing.type: Easing.InOutQuad }
                        }
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        Layout.maximumWidth: Theme.px(330)
                        spacing: 0
                        Line {
                            Layout.fillWidth: true
                            text: win.media ? win.media.title : ""
                            size: 14
                        }
                        Line {
                            Layout.fillWidth: true
                            visible: text.length > 0
                            text: win.media ? (win.media.artist || win.media.app || "") : ""
                            muted: true
                            size: 12
                        }
                    }
                    NotchEqualizer {
                        Layout.preferredHeight: Theme.px(18)
                        playing: win.musicOn
                    }
                }
            }

            Pane {
                name: "listen"
                RowLayout {
                    id: listenRow
                    anchors.fill: parent
                    spacing: Theme.px(12)
                    Eyes {
                        Layout.preferredWidth: Theme.px(40)
                        Layout.preferredHeight: Theme.px(20)
                        mood: win.mood
                        level: listenWave.level
                    }
                    Line {
                        Layout.fillWidth: true
                        Layout.maximumWidth: Theme.px(400)
                        text: win.partial.length > 0 ? win.partial : qsTr("Listening")
                        muted: win.partial.length === 0
                        wrapMode: Text.Wrap
                        maximumLineCount: 2
                    }
                    NotchWave {
                        id: listenWave
                        mode: "listen"
                        Layout.preferredWidth: Theme.px(64)
                        Layout.preferredHeight: Theme.px(24)
                        live: win.view === "listen" && win.shown
                    }
                }
            }

            Pane {
                name: "think"
                RowLayout {
                    id: thinkRow
                    anchors.fill: parent
                    spacing: Theme.px(12)
                    Eyes {
                        Layout.preferredWidth: Theme.px(40)
                        Layout.preferredHeight: Theme.px(20)
                        mood: win.mood
                    }
                    Line {
                        Layout.fillWidth: true
                        text: win.progress ? qsTr("Working, step %1 of %2").arg(win.progress.step).arg(win.progress.total) : qsTr("Working on it")
                    }
                    Row {
                        spacing: Theme.px(4)
                        Repeater {
                            model: 3
                            Rectangle {
                                id: dot
                                required property int index
                                width: Theme.px(6)
                                height: width
                                radius: width / 2
                                color: Theme.notchInk
                                opacity: 0.35
                                SequentialAnimation on opacity {
                                    running: win.view === "think" && !win.calm
                                    loops: Animation.Infinite
                                    PauseAnimation { duration: dot.index * 140 }
                                    NumberAnimation { from: 0.3; to: 1; duration: 260 }
                                    NumberAnimation { from: 1; to: 0.3; duration: 260 }
                                    PauseAnimation { duration: (2 - dot.index) * 140 }
                                }
                                SequentialAnimation on y {
                                    running: win.view === "think" && !win.calm
                                    loops: Animation.Infinite
                                    PauseAnimation { duration: dot.index * 140 }
                                    NumberAnimation { from: 0; to: -Theme.px(4); duration: 260; easing.type: Easing.OutQuad }
                                    NumberAnimation { from: -Theme.px(4); to: 0; duration: 260; easing.type: Easing.InQuad }
                                    PauseAnimation { duration: (2 - dot.index) * 140 }
                                }
                            }
                        }
                    }
                }
            }

            Pane {
                name: "act"
                RowLayout {
                    id: actRow
                    anchors.fill: parent
                    spacing: Theme.px(12)
                    Eyes {
                        Layout.preferredWidth: Theme.px(40)
                        Layout.preferredHeight: Theme.px(20)
                        mood: win.mood
                    }
                    Line {
                        Layout.fillWidth: true
                        text: win.action.label || win.action.target || qsTr("Working on it")
                    }
                    NotchBadge {
                        id: appBadge
                        visible: (win.action.verb === "open_app" || win.action.verb === "close_app") && (win.action.target || "").length > 0
                        name: win.action.target || ""
                        Layout.preferredWidth: Theme.px(26)
                        Layout.preferredHeight: Theme.px(26)
                        scale: 1
                        Connections {
                            target: win
                            function onActionChanged() {
                                if (appBadge.visible && !win.calm)
                                    badgePop.restart()
                            }
                        }
                        SequentialAnimation {
                            id: badgePop
                            NumberAnimation { target: appBadge; property: "scale"; from: 0.2; to: 1.18; duration: 220; easing.type: Easing.OutBack; easing.overshoot: 2.6 }
                            NumberAnimation { target: appBadge; property: "scale"; to: 1; duration: 200; easing.type: Easing.InOutQuad }
                        }
                    }
                }
            }

            Pane {
                name: "speak"
                RowLayout {
                    id: speakRow
                    anchors.fill: parent
                    spacing: Theme.px(12)
                    Eyes {
                        Layout.preferredWidth: Theme.px(40)
                        Layout.preferredHeight: Theme.px(20)
                        mood: win.mood
                        level: speakWave.level
                    }
                    Line {
                        Layout.fillWidth: true
                        Layout.maximumWidth: Theme.px(400)
                        text: win.reply ? (win.reply.lines.length > 0 ? win.reply.lines[Math.min(win.replyLine, win.reply.lines.length - 1)] : win.reply.text) : ""
                        wrapMode: Text.Wrap
                        maximumLineCount: 2
                        font.weight: Font.Medium
                    }
                    NotchWave {
                        id: speakWave
                        mode: "speak"
                        Layout.preferredWidth: Theme.px(64)
                        Layout.preferredHeight: Theme.px(24)
                        live: win.view === "speak" && win.shown
                    }
                }
            }

            Pane {
                name: "done"
                RowLayout {
                    id: doneRow
                    anchors.fill: parent
                    spacing: Theme.px(12)
                    Eyes {
                        Layout.preferredWidth: Theme.px(40)
                        Layout.preferredHeight: Theme.px(20)
                        mood: win.mood
                    }
                    Line {
                        Layout.fillWidth: true
                        Layout.maximumWidth: Theme.px(380)
                        text: win.doneText.length > 0 ? win.doneText : qsTr("Done")
                    }
                    NotchButton {
                        visible: win.action.undoable === true
                        text: qsTr("Undo")
                        implicitHeight: Theme.px(28)
                        onClicked: Daemon.call("action.undo", { id: win.action.id }, (ok, r, e) => {
                            win.doneText = ok ? r.message : e.message
                            win.action = ({})
                            doneTimer.restart()
                        })
                    }
                    Canvas {
                        id: tick
                        property real draw: 0
                        Layout.preferredWidth: Theme.px(18)
                        Layout.preferredHeight: Theme.px(18)
                        renderTarget: Canvas.Image
                        Accessible.ignored: true
                        onDrawChanged: requestPaint()
                        Connections {
                            target: win
                            function onViewChanged() {
                                if (win.view === "done") {
                                    tick.draw = win.calm ? 1 : 0
                                    tickAnim.restart()
                                }
                            }
                        }
                        NumberAnimation { id: tickAnim; target: tick; property: "draw"; to: 1; duration: 360; easing.type: Easing.OutCubic }
                        onPaint: {
                            const ctx = getContext("2d")
                            ctx.reset()
                            const pts = [[0.16, 0.54], [0.42, 0.78], [0.86, 0.26]]
                            const l1 = Math.hypot(pts[1][0] - pts[0][0], pts[1][1] - pts[0][1])
                            const l2 = Math.hypot(pts[2][0] - pts[1][0], pts[2][1] - pts[1][1])
                            const total = (l1 + l2) * draw
                            ctx.strokeStyle = Theme.notchDone
                            ctx.lineWidth = Math.max(2, width * 0.16)
                            ctx.lineCap = "round"
                            ctx.lineJoin = "round"
                            ctx.beginPath()
                            ctx.moveTo(pts[0][0] * width, pts[0][1] * height)
                            if (total <= l1) {
                                const k = total / l1
                                ctx.lineTo((pts[0][0] + (pts[1][0] - pts[0][0]) * k) * width, (pts[0][1] + (pts[1][1] - pts[0][1]) * k) * height)
                            } else {
                                const k = (total - l1) / l2
                                ctx.lineTo(pts[1][0] * width, pts[1][1] * height)
                                ctx.lineTo((pts[1][0] + (pts[2][0] - pts[1][0]) * k) * width, (pts[1][1] + (pts[2][1] - pts[1][1]) * k) * height)
                            }
                            ctx.stroke()
                        }
                    }
                }
            }

            Pane {
                name: "hud"
                RowLayout {
                    anchors.fill: parent
                    spacing: Theme.px(12)
                    Eyes {
                        Layout.preferredWidth: Theme.px(40)
                        Layout.preferredHeight: Theme.px(20)
                        mood: win.mood
                    }
                    Icon {
                        name: win.hud && win.hud.kind === "brightness" ? "sun" : (win.hud && win.hud.muted ? "volume-x" : "volume-2")
                        color: Theme.notchInk
                        size: Theme.px(16)
                    }
                    Line {
                        text: win.hud && win.hud.kind === "brightness" ? qsTr("Brightness") : qsTr("Volume")
                    }
                    Rectangle {
                        Layout.fillWidth: true
                        Layout.preferredHeight: Theme.px(6)
                        radius: height / 2
                        color: Theme.notchRaised
                        Rectangle {
                            height: parent.height
                            radius: parent.radius
                            color: Theme.notchInk
                            width: parent.width * Math.max(0, Math.min(1, (win.hud && win.hud.value >= 0 ? win.hud.value : 0) / 100))
                            Behavior on width {
                                enabled: !win.calm
                                NumberAnimation { duration: 380; easing.type: Easing.OutBack }
                            }
                        }
                    }
                    Line {
                        text: win.hud && win.hud.value >= 0 ? String(win.hud.value) : ""
                        font.features: ({ "tnum": 1 })
                    }
                }
            }

            Pane {
                name: "ask"
                ColumnLayout {
                    id: askCol
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: parent.top
                    spacing: Theme.px(12)
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: Theme.px(12)
                        Face {
                            mood: win.mood
                        }
                        Line {
                            Layout.fillWidth: true
                            text: win.ask ? win.ask.question : ""
                            wrapMode: Text.Wrap
                            maximumLineCount: 3
                            size: 16
                            Accessible.role: Accessible.Heading
                        }
                    }
                    Row {
                        id: logoRow
                        Layout.alignment: Qt.AlignHCenter
                        visible: win.ask !== null && win.ask.kind === "browsers"
                        spacing: Theme.px(14)
                        Repeater {
                            model: logoRow.visible ? win.ask.options : []
                            Item {
                                id: logo
                                required property var modelData
                                required property int index
                                width: Theme.px(76)
                                height: Theme.px(96)
                                activeFocusOnTab: true
                                Accessible.role: Accessible.Button
                                Accessible.name: modelData.label
                                Accessible.onPressAction: logo.pick()
                                Keys.onReturnPressed: logo.pick()
                                Keys.onSpacePressed: logo.pick()
                                function pick() {
                                    Daemon.call("sound.play", { name: "tap" })
                                    Daemon.call("prompt.answer", { id: win.ask.id, answer: logo.modelData.n, via: "click" })
                                }
                                property real bob: 0
                                scale: 0.3
                                opacity: 0
                                Component.onCompleted: arrive.start()
                                SequentialAnimation {
                                    id: arrive
                                    PauseAnimation { duration: win.calm ? 0 : logo.index * 90 }
                                    ParallelAnimation {
                                        NumberAnimation { target: logo; property: "opacity"; to: 1; duration: win.calm ? 0 : 220 }
                                        NumberAnimation { target: logo; property: "scale"; to: 1; duration: win.calm ? 0 : 520; easing.type: Easing.OutBack; easing.overshoot: 2.4 }
                                    }
                                }
                                SequentialAnimation on bob {
                                    running: !win.calm && win.view === "ask"
                                    loops: Animation.Infinite
                                    PauseAnimation { duration: logo.index * 160 }
                                    NumberAnimation { from: 0; to: -Theme.px(5); duration: 700; easing.type: Easing.InOutSine }
                                    NumberAnimation { from: -Theme.px(5); to: 0; duration: 700; easing.type: Easing.InOutSine }
                                }
                                Rectangle {
                                    id: tile
                                    anchors.horizontalCenter: parent.horizontalCenter
                                    y: logo.bob
                                    width: Theme.px(60)
                                    height: width
                                    radius: Theme.px(16)
                                    color: logoHover.hovered || logo.activeFocus ? Theme.notchRaised : Theme.notchCard
                                    border.width: logo.activeFocus || win.scanIndex === logo.index ? 2 : 0
                                    border.color: Theme.notchInk
                                    scale: logoHover.hovered ? 1.08 : 1
                                    Behavior on scale {
                                        NumberAnimation { duration: 160; easing.type: Easing.OutBack }
                                    }
                                    Image {
                                        id: logoImage
                                        anchors.centerIn: parent
                                        width: Theme.px(40)
                                        height: width
                                        sourceSize: Qt.size(width * 2, height * 2)
                                        source: logo.modelData.icon && logo.modelData.icon.length > 0 ? "image://appicon/" + encodeURIComponent(logo.modelData.icon) : ""
                                        fillMode: Image.PreserveAspectFit
                                        smooth: true
                                        Accessible.ignored: true
                                    }
                                    Icon {
                                        anchors.centerIn: parent
                                        visible: logoImage.status !== Image.Ready
                                        name: "globe"
                                        size: Theme.px(30)
                                        color: Theme.notchInk
                                    }
                                    HoverHandler {
                                        id: logoHover
                                        cursorShape: Qt.PointingHandCursor
                                    }
                                    TapHandler {
                                        onTapped: logo.pick()
                                    }
                                }
                                Line {
                                    anchors.horizontalCenter: parent.horizontalCenter
                                    anchors.bottom: parent.bottom
                                    width: parent.width
                                    horizontalAlignment: Text.AlignHCenter
                                    text: logo.modelData.label
                                    size: 12
                                    muted: !logoHover.hovered
                                }
                            }
                        }
                    }
                    ListView {
                        id: profileStrip
                        Layout.fillWidth: true
                        Layout.preferredHeight: Theme.px(112)
                        visible: win.ask !== null && (win.ask.kind === "profiles" || win.ask.kind === "folders")
                        orientation: ListView.Horizontal
                        interactive: false
                        clip: true
                        spacing: Theme.px(10)
                        model: visible ? win.ask.options : []
                        currentIndex: visible && win.ask.focus !== undefined && win.ask.focus !== null ? win.ask.focus : 0
                        highlightRangeMode: ListView.StrictlyEnforceRange
                        preferredHighlightBegin: width / 2 - Theme.px(42)
                        preferredHighlightEnd: width / 2 + Theme.px(42)
                        highlightMoveDuration: win.calm ? 0 : 420
                        Accessible.role: Accessible.List
                        Accessible.name: qsTr("Profiles")
                        delegate: Item {
                            id: card
                            required property var modelData
                            required property int index
                            readonly property bool chosen: ListView.isCurrentItem
                            width: Theme.px(84)
                            height: profileStrip.height
                            activeFocusOnTab: true
                            Accessible.role: Accessible.Button
                            Accessible.name: modelData.label
                            Accessible.onPressAction: card.pick()
                            Keys.onReturnPressed: card.pick()
                            Keys.onSpacePressed: card.pick()
                            function pick() {
                                Daemon.call("sound.play", { name: "tap" })
                                Daemon.call("prompt.answer", { id: win.ask.id, answer: card.modelData.n, via: "click" })
                            }
                            Rectangle {
                                id: face
                                anchors.horizontalCenter: parent.horizontalCenter
                                y: Theme.px(6)
                                width: Theme.px(58)
                                height: width
                                radius: width / 2
                                color: card.chosen ? Theme.notchInk : Theme.notchCard
                                border.width: card.activeFocus ? 2 : 0
                                border.color: Theme.notchInk
                                scale: card.chosen ? 1.14 : 0.86
                                opacity: card.chosen ? 1 : 0.62
                                Behavior on scale {
                                    NumberAnimation { duration: win.calm ? 0 : 420; easing.type: Easing.OutBack; easing.overshoot: 2.2 }
                                }
                                Behavior on opacity {
                                    NumberAnimation { duration: win.calm ? 0 : 200 }
                                }
                                Line {
                                    anchors.centerIn: parent
                                    visible: !cardLook.visible
                                    text: String(card.modelData.label || "?").trim().charAt(0).toUpperCase()
                                    size: 24
                                    color: card.chosen ? Theme.notchCard : Theme.notchInk
                                }
                                LookAvatar {
                                    id: cardLook
                                    readonly property var parts: String(card.modelData.icon || "").split("|")
                                    anchors.fill: parent
                                    anchors.margins: 2
                                    visible: parts[0] === "look"
                                    avatar: visible ? (parts[1] || "") : ""
                                    file: visible ? (parts[2] || "") : ""
                                }
                                TapHandler {
                                    onTapped: card.pick()
                                }
                            }
                            Rectangle {
                                anchors.horizontalCenter: face.horizontalCenter
                                anchors.top: face.bottom
                                anchors.topMargin: Theme.px(6)
                                width: Theme.px(6)
                                height: width
                                radius: width / 2
                                color: Theme.notchInk
                                visible: String(card.modelData.icon || "").endsWith("current")
                            }
                            Line {
                                anchors.horizontalCenter: parent.horizontalCenter
                                anchors.bottom: parent.bottom
                                width: parent.width
                                horizontalAlignment: Text.AlignHCenter
                                elide: Text.ElideRight
                                text: card.modelData.label
                                size: 12
                                muted: !card.chosen
                            }
                        }
                    }
                    Flow {
                        Layout.fillWidth: true
                        visible: !logoRow.visible && !profileStrip.visible && !(win.ask && String(win.ask.kind || "").startsWith("look"))
                        spacing: Theme.px(8)
                        Repeater {
                            id: optionRepeater
                            model: win.ask && win.ask.kind !== "browsers" && win.ask.kind !== "profiles" && win.ask.kind !== "folders" ? win.ask.options : []
                            NotchButton {
                                required property var modelData
                                required property int index
                                text: modelData.n + "  " + modelData.label
                                variant: index === 0 ? "primary" : "secondary"
                                onClicked: Daemon.call("prompt.answer", { id: win.ask.id, answer: modelData.n, via: "click" })
                            }
                        }
                    }
                    Line {
                        Layout.fillWidth: true
                        muted: true
                        size: 12
                        horizontalAlignment: logoRow.visible ? Text.AlignHCenter : Text.AlignLeft
                        text: logoRow.visible ? qsTr("Say its name, or click one. I'll set up the rest.") : profileStrip.visible ? qsTr("Say left, right, or a name") : qsTr("Say the option or its number")
                    }
                }
            }

            Pane {
                name: "confirm"
                ColumnLayout {
                    id: confirmCol
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: parent.top
                    spacing: Theme.px(10)
                    Card {
                        Layout.fillWidth: true
                        Face {
                            mood: win.mood
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: Theme.px(4)
                            Line {
                                Layout.fillWidth: true
                                text: win.local ? win.local.title : (win.confirm ? win.confirm.title : "")
                                wrapMode: Text.Wrap
                                maximumLineCount: 3
                                size: 16
                                Accessible.role: Accessible.Heading
                            }
                            Line {
                                Layout.fillWidth: true
                                visible: text.length > 0
                                text: win.local ? win.local.consequence : (win.confirm ? win.confirm.consequence : "")
                                muted: true
                                size: 13
                                wrapMode: Text.Wrap
                                maximumLineCount: 3
                            }
                            Repeater {
                                model: win.confirm && !win.local ? win.confirm.facts.slice(0, 3) : []
                                Line {
                                    required property string modelData
                                    Layout.fillWidth: true
                                    text: modelData
                                    muted: true
                                    size: 12
                                    font.family: Type.mono
                                }
                            }
                        }
                        NotchButton {
                            id: noButton
                            text: qsTr("Not now")
                            onClicked: win.answerConfirm(false, "click")
                        }
                        NotchButton {
                            id: yesButton
                            text: win.local ? win.local.actionLabel : qsTr("Yes")
                            variant: (win.local || (win.confirm && win.confirm.danger)) ? "danger" : "primary"
                            onClicked: win.answerConfirm(true, "click")
                        }
                    }
                    Line {
                        Layout.fillWidth: true
                        visible: win.confirm !== null && win.confirm.voiceAllowed === true
                        muted: true
                        size: 12
                        horizontalAlignment: Text.AlignHCenter
                        text: qsTr("Say yes or no")
                    }
                }
            }

            Pane {
                name: "problem"
                ColumnLayout {
                    id: problemCol
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: parent.top
                    spacing: Theme.px(10)
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: Theme.px(12)
                        Face {
                            mood: "sad"
                        }
                        Line {
                            Layout.fillWidth: true
                            text: win.problemText
                            wrapMode: Text.Wrap
                            maximumLineCount: 4
                            font.weight: Font.Medium
                        }
                    }
                    RowLayout {
                        Layout.alignment: Qt.AlignRight
                        spacing: Theme.px(8)
                        NotchButton {
                            id: fixButton
                            visible: win.problemAction !== null && win.problemAction !== undefined
                            text: win.problemAction ? win.problemAction.label : ""
                            variant: "primary"
                            onClicked: {
                                const a = win.problemAction
                                if (a.method === "ui.open")
                                    Launcher.openWindow(a.params.page)
                                else
                                    Daemon.call(a.method, a.params || {})
                                win.hide()
                            }
                        }
                        NotchButton {
                            id: dismissButton
                            text: qsTr("Dismiss")
                            onClicked: win.hide()
                        }
                    }
                }
            }

            Pane {
                name: "sounds"
                ColumnLayout {
                    id: soundsCol
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: parent.top
                    spacing: Theme.px(8)
                    Line {
                        text: qsTr("Playing now")
                        muted: true
                        size: 13
                    }
                    Repeater {
                        model: win.sounds
                        delegate: Card {
                            id: soundRow
                            required property var modelData
                            required property int index
                            Layout.fillWidth: true
                            gap: Theme.px(10)
                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: Theme.px(2)
                                Line {
                                    Layout.fillWidth: true
                                    text: (soundRow.index + 1) + ". " + soundRow.modelData.app
                                    elide: Text.ElideRight
                                    size: 14
                                }
                                Line {
                                    Layout.fillWidth: true
                                    visible: text.length > 0
                                    text: soundRow.modelData.muted ? qsTr("Muted") : (soundRow.modelData.title || "")
                                    elide: Text.ElideRight
                                    muted: true
                                    size: 12
                                }
                            }
                            NotchButton {
                                readonly property bool on: soundRow.modelData.playing === true
                                Accessible.name: on ? (soundRow.modelData.stream ? qsTr("Mute %1").arg(soundRow.modelData.app) : qsTr("Pause %1").arg(soundRow.modelData.app)) : qsTr("Resume %1").arg(soundRow.modelData.app)
                                iconName: on ? "pause" : "play"
                                variant: on ? "primary" : "secondary"
                                onClicked: {
                                    Daemon.call("sound.play", { name: "tap" })
                                    Daemon.call("media.control", { key: soundRow.modelData.key, action: on ? "pause" : "play" })
                                }
                            }
                        }
                    }
                }
            }

            Pane {
                name: "home"
                ColumnLayout {
                    id: homeCol
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: parent.top
                    spacing: Theme.px(12)
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: Theme.px(8)
                        NotchButton {
                            Accessible.name: win.muted ? qsTr("Turn the microphone on") : qsTr("Mute microphone")
                            iconName: win.muted ? "mic-off" : "mic"
                            variant: win.muted ? "secondary" : "primary"
                            onClicked: Daemon.call("mic.setMuted", { muted: !win.muted })
                        }
                        NotchButton {
                            Accessible.name: qsTr("Open %1").arg(Brand.productName)
                            iconName: "app-window"
                            onClicked: Launcher.openWindow("home")
                        }
                        NotchButton {
                            Accessible.name: qsTr("Settings")
                            iconName: "settings"
                            onClicked: Launcher.openWindow("settings/voice")
                        }
                        Item { Layout.fillWidth: true }
                        Line {
                            text: Qt.formatDateTime(new Date(win.now), "ddd d MMM  h:mm")
                            muted: true
                            size: 13
                        }
                    }
                    Card {
                        id: mediaCard
                        Layout.fillWidth: true
                        visible: win.media !== null
                        readonly property bool timed: win.media !== null && win.media.durationMs > 0 && win.mediaPos >= 0
                        readonly property var lyrics: win.media && win.media.lyrics ? win.media.lyrics : []
                        NotchArt {
                            Layout.preferredWidth: Theme.px(64)
                            Layout.preferredHeight: Theme.px(64)
                            Layout.alignment: Qt.AlignTop
                            radius: Theme.px(12)
                            source: win.media && win.media.art ? win.media.art : ""
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            Layout.preferredWidth: Theme.px(190)
                            spacing: Theme.px(3)
                            Line {
                                Layout.fillWidth: true
                                text: win.media ? win.media.title : ""
                                size: 15
                            }
                            Line {
                                Layout.fillWidth: true
                                visible: text.length > 0
                                text: win.media ? (win.media.artist || win.media.app || "") : ""
                                muted: true
                                size: 12
                            }
                            Rectangle {
                                Layout.fillWidth: true
                                Layout.topMargin: Theme.px(6)
                                Layout.preferredHeight: Theme.px(4)
                                visible: mediaCard.timed
                                radius: height / 2
                                color: Theme.notchRaised
                                Rectangle {
                                    height: parent.height
                                    radius: parent.radius
                                    color: Theme.notchInk
                                    width: parent.width * (mediaCard.timed ? Math.max(0, Math.min(1, win.mediaPos / win.media.durationMs)) : 0)
                                }
                            }
                            RowLayout {
                                Layout.fillWidth: true
                                visible: mediaCard.timed
                                Line {
                                    text: mediaCard.timed ? win.clock(win.mediaPos) : ""
                                    muted: true
                                    size: 11
                                    font.features: ({ "tnum": 1 })
                                }
                                Item { Layout.fillWidth: true }
                                Line {
                                    text: mediaCard.timed ? "-" + win.clock(Math.max(0, win.media.durationMs - win.mediaPos)) : ""
                                    muted: true
                                    size: 11
                                    font.features: ({ "tnum": 1 })
                                }
                            }
                            RowLayout {
                                Layout.alignment: Qt.AlignHCenter
                                spacing: Theme.px(10)
                                NotchButton {
                                    Accessible.name: qsTr("Previous")
                                    iconName: "skip-back"
                                    implicitHeight: Theme.px(30)
                                    enabled: win.media !== null && win.media.canPrev !== false
                                    onClicked: win.control("previous")
                                }
                                NotchButton {
                                    Accessible.name: win.musicOn ? qsTr("Pause") : qsTr("Play")
                                    iconName: win.musicOn ? "pause" : "play"
                                    variant: "primary"
                                    implicitHeight: Theme.px(34)
                                    onClicked: win.control(win.musicOn ? "pause" : "play")
                                }
                                NotchButton {
                                    Accessible.name: qsTr("Next")
                                    iconName: "skip-forward"
                                    implicitHeight: Theme.px(30)
                                    enabled: win.media !== null && win.media.canNext !== false
                                    onClicked: win.control("next")
                                }
                            }
                        }
                        Rectangle {
                            Layout.fillHeight: true
                            Layout.preferredWidth: 1
                            visible: mediaCard.lyrics.length > 0
                            color: Theme.notchRaised
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            Layout.preferredWidth: Theme.px(200)
                            visible: mediaCard.lyrics.length > 0
                            spacing: Theme.px(4)
                            Line {
                                id: lyricNow
                                Layout.fillWidth: true
                                text: win.lyricIndex >= 0 ? mediaCard.lyrics[win.lyricIndex][1] : (mediaCard.lyrics.length > 0 ? mediaCard.lyrics[0][1] : "")
                                size: 15
                                wrapMode: Text.Wrap
                                maximumLineCount: 2
                                opacity: win.lyricIndex >= 0 ? 1 : 0.6
                                Behavior on text {
                                    enabled: !win.calm
                                    SequentialAnimation {
                                        NumberAnimation { target: lyricNow; property: "opacity"; to: 0; duration: 90 }
                                        PropertyAction {}
                                        NumberAnimation { target: lyricNow; property: "opacity"; to: 1; duration: 160 }
                                    }
                                }
                            }
                            Line {
                                Layout.fillWidth: true
                                text: win.lyricIndex + 1 < mediaCard.lyrics.length ? mediaCard.lyrics[win.lyricIndex + 1][1] : ""
                                muted: true
                                size: 13
                                wrapMode: Text.Wrap
                                maximumLineCount: 2
                            }
                        }
                    }
                    Card {
                        Layout.fillWidth: true
                        visible: win.focusTimer !== null
                        Face {
                            mood: win.focusTimer && win.focusTimer.kind === "focus" ? "focus" : "happy"
                        }
                        NotchRing {
                            Layout.preferredWidth: Theme.px(64)
                            Layout.preferredHeight: Theme.px(64)
                            thickness: Theme.px(5)
                            stroke: win.focusTimer && win.focusTimer.kind === "break" ? Theme.notchBreak : Theme.notchFocus
                            progress: win.done(win.focusTimer)
                            glyph: "clock"
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 0
                            Line {
                                text: win.clock(win.left(win.focusTimer))
                                size: 30
                                font.weight: Font.Bold
                                font.features: ({ "tnum": 1 })
                            }
                            Line {
                                text: win.focusTimer && win.focusTimer.kind === "break" ? qsTr("Break") : qsTr("Focus time")
                                muted: true
                                size: 13
                            }
                        }
                        NotchButton {
                            text: win.focusTimer && win.focusTimer.kind === "break" ? qsTr("Focus") : qsTr("Break")
                            onClicked: Daemon.call("notch.action", { action: win.focusTimer && win.focusTimer.kind === "break" ? "focus.start" : "focus.break" })
                        }
                        NotchButton {
                            Accessible.name: qsTr("Stop")
                            stopGlyph: true
                            variant: "focus"
                            tint: win.focusTimer && win.focusTimer.kind === "break" ? Theme.notchBreak : Theme.notchFocus
                            onClicked: Daemon.call("notch.action", { action: "focus.stop" })
                        }
                    }
                    Card {
                        Layout.fillWidth: true
                        visible: win.focusTimer === null && win.countdown !== null
                        NotchRing {
                            Layout.preferredWidth: Theme.px(52)
                            Layout.preferredHeight: Theme.px(52)
                            thickness: Theme.px(5)
                            stroke: Theme.notchTimer
                            progress: win.done(win.countdown)
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 0
                            Line {
                                text: win.clock(win.left(win.countdown))
                                size: 28
                                font.weight: Font.Bold
                                font.features: ({ "tnum": 1 })
                            }
                            Line {
                                Layout.fillWidth: true
                                text: win.countdown && win.countdown.label.length > 0 ? win.countdown.label : qsTr("Timer")
                                muted: true
                                size: 13
                            }
                        }
                        NotchButton {
                            Accessible.name: qsTr("Cancel the timer")
                            iconName: "x"
                            onClicked: Daemon.call("notch.action", { action: "timer.cancel", id: win.countdown ? win.countdown.id : 0 })
                        }
                    }
                    Card {
                        Layout.fillWidth: true
                        visible: win.focusTimer === null && win.countdown === null && win.watch !== null
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 0
                            Line {
                                text: qsTr("Stopwatch")
                                color: Theme.notchTimer
                                size: 13
                            }
                            Line {
                                text: win.clock(win.watchMs())
                                size: 28
                                font.weight: Font.Bold
                                font.features: ({ "tnum": 1 })
                            }
                        }
                        NotchButton {
                            text: win.watch && win.watch.running ? qsTr("Stop") : qsTr("Start")
                            variant: "primary"
                            onClicked: Daemon.call("notch.action", { action: win.watch && win.watch.running ? "stopwatch.stop" : "stopwatch.start" })
                        }
                        NotchButton {
                            text: qsTr("Reset")
                            onClicked: Daemon.call("notch.action", { action: "stopwatch.reset" })
                        }
                    }
                    Card {
                        Layout.fillWidth: true
                        visible: win.focusTimer === null && win.countdown === null && win.watch === null && win.media === null
                        Face {
                            mood: win.awake ? "happy" : "idle"
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: Theme.px(2)
                            Line {
                                Layout.fillWidth: true
                                text: win.muted ? qsTr("Microphone is off") : qsTr("Say “%1”").arg(Brand.wakePhrase)
                                size: 16
                            }
                            Line {
                                Layout.fillWidth: true
                                text: win.reminder ? qsTr("Next: %1 at %2").arg(win.reminder.text).arg(Qt.formatTime(new Date(win.reminder.at), "h:mm ap")) : (win.todos.total > 0 ? qsTr("%1 of %2 done today").arg(win.todos.doneToday).arg(win.todos.total) : qsTr("Timers, notes and to-dos live here"))
                                muted: true
                                size: 13
                            }
                        }
                        NotchRing {
                            visible: win.todos.total > 0
                            Layout.preferredWidth: Theme.px(34)
                            Layout.preferredHeight: Theme.px(34)
                            thickness: Theme.px(4)
                            stroke: Theme.notchDone
                            progress: win.todos.total > 0 ? win.todos.doneToday / win.todos.total : 0
                            glyph: "check"
                        }
                    }
                }
            }

            Pane {
                name: "peek"
                ColumnLayout {
                    id: peekCol
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: parent.top
                    spacing: Theme.px(10)
                    Card {
                        Layout.fillWidth: true
                        visible: win.peek !== null && win.peek.kind === "notes"
                        Face {
                            mood: "happy"
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: Theme.px(6)
                            Line {
                                text: qsTr("Notes")
                                size: 15
                                Accessible.role: Accessible.Heading
                            }
                            Repeater {
                                model: (win.board.notes || []).slice(0, 3)
                                RowLayout {
                                    id: noteRow
                                    required property var modelData
                                    Layout.fillWidth: true
                                    spacing: Theme.px(8)
                                    Rectangle {
                                        Layout.preferredWidth: Theme.px(12)
                                        Layout.preferredHeight: Theme.px(12)
                                        radius: width / 2
                                        color: "transparent"
                                        border.width: 2
                                        border.color: Theme.notchFaint
                                    }
                                    Line {
                                        Layout.fillWidth: true
                                        text: noteRow.modelData.text
                                        font.weight: Font.Normal
                                        size: 14
                                    }
                                }
                            }
                            Line {
                                visible: (win.board.notes || []).length === 0
                                text: qsTr("No notes yet. Say “take a note”.")
                                muted: true
                                size: 13
                            }
                        }
                    }
                    Card {
                        Layout.fillWidth: true
                        visible: win.peek !== null && win.peek.kind === "todos"
                        ColumnLayout {
                            Layout.preferredWidth: Theme.px(110)
                            spacing: 0
                            Face {
                                Layout.alignment: Qt.AlignHCenter
                                implicitWidth: Theme.px(76)
                                implicitHeight: Theme.px(40)
                                mood: "happy"
                            }
                            Line {
                                Layout.alignment: Qt.AlignHCenter
                                text: win.todos.doneToday + "/" + Math.max(win.todos.total, win.todos.doneToday)
                                size: 30
                                font.weight: Font.Bold
                            }
                            Line {
                                Layout.alignment: Qt.AlignHCenter
                                text: qsTr("Done today")
                                muted: true
                                size: 12
                            }
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: Theme.px(4)
                            Repeater {
                                model: (win.todos.items || []).slice(0, 5)
                                Rectangle {
                                    id: todoRow
                                    required property var modelData
                                    Layout.fillWidth: true
                                    implicitHeight: Theme.px(26)
                                    radius: Theme.px(8)
                                    color: modelData.done ? Qt.rgba(0.19, 0.82, 0.35, 0.18) : Theme.notchRaised
                                    Accessible.role: Accessible.CheckBox
                                    Accessible.name: modelData.text
                                    Accessible.checked: modelData.done
                                    Accessible.onToggleAction: toggle()
                                    Accessible.onPressAction: toggle()
                                    function toggle() {
                                        Daemon.call("notch.action", { action: "todo.toggle", id: modelData.id })
                                    }
                                    RowLayout {
                                        anchors.fill: parent
                                        anchors.leftMargin: Theme.px(8)
                                        anchors.rightMargin: Theme.px(8)
                                        spacing: Theme.px(8)
                                        Rectangle {
                                            Layout.preferredWidth: Theme.px(14)
                                            Layout.preferredHeight: Theme.px(14)
                                            radius: width / 2
                                            color: todoRow.modelData.done ? Theme.notchDone : "transparent"
                                            border.width: todoRow.modelData.done ? 0 : 2
                                            border.color: Theme.notchFaint
                                            Icon {
                                                anchors.centerIn: parent
                                                visible: todoRow.modelData.done
                                                name: "check"
                                                color: Theme.notchBg
                                                size: Theme.px(10)
                                            }
                                        }
                                        Line {
                                            Layout.fillWidth: true
                                            text: todoRow.modelData.text
                                            muted: todoRow.modelData.done
                                            font.strikeout: todoRow.modelData.done
                                            font.weight: Font.Normal
                                            size: 13
                                        }
                                    }
                                    TapHandler {
                                        onTapped: todoRow.toggle()
                                    }
                                }
                            }
                            Line {
                                visible: (win.todos.items || []).length === 0
                                text: qsTr("Nothing to do. Say “add buy milk to my to-do list”.")
                                muted: true
                                size: 13
                                wrapMode: Text.Wrap
                                Layout.fillWidth: true
                            }
                        }
                    }
                    Card {
                        Layout.fillWidth: true
                        visible: win.peek !== null && ["timer", "focus", "break", "reminder", "install"].includes(win.peek.kind)
                        Face {
                            mood: win.peek && win.peek.kind === "install" ? (win.peek.ok ? "happy" : "sad") : "wow"
                        }
                        Line {
                            Layout.fillWidth: true
                            text: win.peek ? win.peek.text : ""
                            wrapMode: Text.Wrap
                            maximumLineCount: 3
                            size: 16
                        }
                        NotchButton {
                            text: qsTr("OK")
                            variant: "primary"
                            onClicked: win.peek = null
                        }
                    }
                }
            }
        }
    }

    Rectangle {
        z: 10
        readonly property var items: win.scanIndex >= 0 ? win.interactiveItems() : []
        readonly property Item target: items.length > 0 ? items[win.scanIndex % items.length] : null
        readonly property point at: target ? target.mapToItem(win.contentItem, 0, 0) : Qt.point(0, 0)
        visible: win.scanning && target !== null
        x: at.x - 4
        y: at.y - 4
        width: target ? target.width + 8 : 0
        height: target ? target.height + 8 : 0
        radius: height / 2
        color: "transparent"
        border.width: 3
        border.color: Theme.notchDone
    }
}
