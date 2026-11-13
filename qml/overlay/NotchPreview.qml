import QtQuick
import Setu.Core
import Setu.ShellHost

QtObject {
    id: preview
    required property var notch
    property int step: -1
    readonly property real t0: Date.now()
    readonly property var steps: [
        { name: "idle", apply: n => {} },
        { name: "listen", apply: n => { n.partial = qsTr("Remind me to call Mom at six"); n.phase = "listening" } },
        { name: "think", apply: n => { n.phase = "thinking" } },
        { name: "act", apply: n => { n.action = { verb: "open_app", target: "Firefox", label: qsTr("Opening Firefox") }; n.phase = "acting" } },
        { name: "done", apply: n => { n.doneText = qsTr("Opened Firefox"); n.phase = "done" } },
        { name: "speak", apply: n => { n.reply = { id: "1", text: qsTr("It's 9:41 in the morning."), lines: [qsTr("It's 9:41 in the morning.")] }; n.phase = "speaking" } },
        { name: "hud", apply: n => { n.action = { verb: "set_volume" }; n.hud = { kind: "volume", value: 65, muted: false }; n.doneText = qsTr("Volume is at 65%"); n.phase = "done" } },
        { name: "confirm", apply: n => { n.hud = null; n.confirm = { id: "c", title: qsTr("Send the slides to Sam?"), consequence: "", facts: [], danger: false, voiceAllowed: true }; n.phase = "confirming" } },
        { name: "ask", apply: n => { n.confirm = null; n.ask = { id: "a", question: qsTr("Which one: Firefox or Chrome?"), options: [{ n: 1, label: qsTr("Firefox") }, { n: 2, label: qsTr("Chrome") }] }; n.phase = "asking" } },
        { name: "browsers", apply: n => { n.ask = { id: "b", kind: "browsers", question: qsTr("Which browser should I use?"), options: [{ n: 1, label: qsTr("Brave"), icon: "brave-browser" }, { n: 2, label: qsTr("Chrome"), icon: "google-chrome" }, { n: 3, label: qsTr("Firefox"), icon: "firefox" }, { n: 4, label: qsTr("Edge"), icon: "microsoft-edge" }] }; n.phase = "asking" } },
        { name: "profiles", apply: n => { n.ask = { id: "p", kind: "profiles", question: qsTr("Which Brave profile?"), focus: 2, options: [{ n: 1, label: qsTr("Anoop"), icon: "" }, { n: 2, label: qsTr("Work"), icon: "" }, { n: 3, label: qsTr("Anoop_setu"), icon: "look|goku_DBZ||current" }, { n: 4, label: qsTr("College"), icon: "" }, { n: 5, label: qsTr("Mehra"), icon: "" }] }; n.phase = "asking" } },
        { name: "profiles-right", apply: n => { n.ask = Object.assign({}, n.ask, { focus: 3 }) } },
        { name: "problem", apply: n => { n.ask = null; n.problemText = qsTr("I couldn't find Photoshop. Install it, or pick another app?"); n.problemAction = { label: qsTr("Install"), method: "intent.run", params: {} }; n.phase = "problem" } },
        { name: "timer", apply: n => { n.hide(); n.board = preview.board([{ id: 1, kind: "timer", label: "", endsAt: preview.t0 + 597000, totalMs: 900000 }], null) } },
        { name: "focus", apply: n => { n.board = preview.board([{ id: 2, kind: "focus", label: "", endsAt: preview.t0 + 1450000, totalMs: 1500000 }], null); n.hovering = true } },
        { name: "break", apply: n => { n.board = preview.board([{ id: 3, kind: "break", label: "", endsAt: preview.t0 + 294000, totalMs: 300000 }], null) } },
        { name: "focusMini", apply: n => { n.hovering = false } },
        { name: "jobs", apply: n => { n.board = preview.board([], null); n.jobs = [{ id: 1, app: "Steam", step: "download", percent: 62, paused: false }] } },
        { name: "jobs-two", apply: n => { n.jobs = [{ id: 1, app: "Steam", step: "download", percent: 62, paused: false }, { id: 2, app: "GIMP", step: "install", percent: null, paused: false }] } },
        { name: "stopwatch", apply: n => { n.jobs = []; n.board = preview.board([], { startedAt: preview.t0 - 84000, carriedMs: 0, running: true }) } },
        { name: "notes", apply: n => { n.board = preview.board([], null); n.peek = { kind: "notes", text: "" } } },
        { name: "todos", apply: n => { n.peek = { kind: "todos", text: "" } } },
        { name: "alert", apply: n => { n.peek = { kind: "timer", text: qsTr("Your 15 minute timer is done.") } } },
        { name: "musicMini", apply: n => { n.peek = null; n.setMedia({ key: "os:demo", app: "Spotify", title: qsTr("Paper Sky"), artist: qsTr("Lumen Coast"), album: "", art: "", playing: true, positionMs: 62000, durationMs: 254000, rate: 1, at: Date.now(), canNext: true, canPrev: true, lyrics: [[60000, qsTr("Into paper birds tonight")], [66000, qsTr("And send them through the sky")]] }); n.trackPeek = false } },
        { name: "track", apply: n => { n.trackPeek = true } },
        { name: "media", apply: n => { n.trackPeek = false; n.hovering = true } },
        { name: "sounds", apply: n => { n.hovering = false; n.setMedia(Object.assign({}, n.media, { listOpen: true, list: [{ key: "os:demo", app: "Spotify", title: qsTr("Paper Sky"), playing: true, muted: false, stream: false }, { key: "stream:7", app: qsTr("Bench Game"), title: qsTr("Level 3"), playing: true, muted: false, stream: true }, { key: "os:vlc", app: "VLC", title: qsTr("Lecture"), playing: false, muted: false, stream: false }] })) } },
        { name: "sounds-done", apply: n => { n.setMedia(Object.assign({}, n.media, { listOpen: false, list: [] })) } },
        { name: "home", apply: n => { n.setMedia({ none: true }); n.hovering = true } },
        { name: "muted", apply: n => { n.hovering = false } }
    ]

    function board(timers, watch) {
        return {
            timers: timers,
            stopwatch: watch,
            notes: [{ id: 1, text: qsTr("Wi-Fi password is on the fridge") }, { id: 2, text: qsTr("Call the bank on Monday") }, { id: 3, text: qsTr("Gift ideas: book, plant") }],
            todos: {
                items: [{ id: 1, text: qsTr("Post weekly update"), done: false }, { id: 2, text: qsTr("Buy milk"), done: false }, { id: 3, text: qsTr("Revise the report"), done: true }, { id: 4, text: qsTr("Reply to Dina"), done: true }, { id: 5, text: qsTr("Book the dentist"), done: true }],
                doneToday: 3,
                total: 5
            },
            reminders: [],
            now: Date.now()
        }
    }

    property Timer driver: Timer {
        interval: 1100
        repeat: true
        running: true
        onTriggered: {
            if (preview.step >= 0)
                OverlayHost.capture(preview.notch, OverlayHost.shots + "/notch-" + preview.steps[preview.step].name + ".png")
            preview.step += 1
            if (preview.step >= preview.steps.length) {
                running = false
                Qt.quit()
                return
            }
            preview.steps[preview.step].apply(preview.notch)
        }
    }
}
