import QtQuick
import QtQml.Models
import Qt.labs.platform as Labs
import Setu.Core
import Setu.Theme
import Setu.ShellHost
import Ledger

QtObject {
    id: shell
    readonly property string product: Brand.productName
    readonly property var session: Daemon.session
    readonly property bool muted: Daemon.mic.muted === true
    readonly property bool cameraOn: camera !== null
    property var recent: []
    property real lastStart: 0
    property int minutesLeft: 0
    property var badges: null
    property var camera: null
    property var mover: null
    property bool setupOffered: false
    readonly property bool setupDone: Settings.values["setup.completed"] === true

    readonly property string trayState: {
        if (Daemon.status === "stopped" || Daemon.status === "refused" || Daemon.status === "mismatch")
            return "stopped"
        if (!Daemon.connected)
            return "starting"
        if (muted)
            return "muted"
        if (Daemon.attention.length > 0)
            return "attention"
        if (notch.phase === "acting" || notch.phase === "thinking")
            return "working"
        if (session.state === "capturing" || session.state === "grace" || session.state === "active")
            return "active"
        return "idle"
    }

    readonly property string tooltip: {
        let base
        switch (trayState) {
        case "stopped": base = qsTr("%1: not running").arg(product); break
        case "starting": base = qsTr("%1: starting").arg(product); break
        case "muted": base = qsTr("%1: microphone off").arg(product); break
        case "attention": base = qsTr("%1: needs attention").arg(product); break
        case "working": base = qsTr("%1: working").arg(product); break
        case "active":
            if (session.mode === "coding") {
                base = qsTr("%1: coding mode. Speak your code, or say stop coding.").arg(product)
                break
            }
            if (session.mode === "dictation") {
                base = qsTr("%1: dictation. It types what you say; say stop dictating to end.").arg(product)
                break
            }
            base = session.state === "active" ? qsTr("%1: active for %n more minutes. Say %2, then your command.", "", minutesLeft).arg(product).arg(Brand.assistantName) : qsTr("%1: listening to you").arg(product)
            break
        default: base = qsTr("%1: listening for %2").arg(product).arg(Brand.wakePhrase)
        }
        return cameraOn ? qsTr("Camera on. %1").arg(base) : base
    }

    function mute(on, minutes) {
        let p = { muted: on }
        if (on && minutes)
            p.minutes = minutes
        Daemon.call("mic.setMuted", p)
    }

    function refreshRecent() {
        Daemon.call("history.query", { limit: 5 }, (ok, r) => {
            if (ok)
                shell.recent = r.items
        })
    }

    function updateMinutes() {
        const until = session.activeUntil
        minutesLeft = until ? Math.max(1, Math.ceil((until - Date.now()) / 60000)) : 0
    }

    property Notch notch: Notch {
        onQuitConfirmed: {
            Daemon.call("daemon.quit", {})
            Qt.callLater(Qt.quit)
        }
    }

    property Component badgesComponent: Component {
        Badges {}
    }
    property Component cameraComponent: Component {
        CameraPreview {}
    }
    property var faceWindow: null
    property var vaultWindow: null
    property var revealWindow: null
    property Component faceComponent: Component {
        FaceSetup {}
    }
    property Component vaultComponent: Component {
        VaultCard {}
    }
    property Component revealComponent: Component {
        VaultReveal {}
    }
    function showFace(d) {
        if (d && d.open && d.mode !== "lock") {
            if (!shell.faceWindow)
                shell.faceWindow = shell.faceComponent.createObject(null, { mode: d.mode })
            if (d.frame)
                shell.faceWindow.frame = d.frame
            if (d.message)
                shell.faceWindow.message = d.message
            if (d.steps)
                shell.faceWindow.steps = d.steps
        } else if (shell.faceWindow && (!d || !d.open)) {
            shell.faceWindow.destroy()
            shell.faceWindow = null
        }
    }
    function showVault(d) {
        if (d && d.open) {
            if (shell.vaultWindow && shell.vaultWindow.entry.id === d.id) {
                shell.vaultWindow.entry = d
            } else {
                if (shell.vaultWindow)
                    shell.vaultWindow.destroy()
                shell.vaultWindow = shell.vaultComponent.createObject(null, { entry: d })
            }
        } else if (shell.vaultWindow && (!d || !d.id || shell.vaultWindow.entry.id === d.id)) {
            shell.vaultWindow.busy = true
            shell.vaultWindow.destroy()
            shell.vaultWindow = null
        }
    }
    function showReveal(d) {
        if (shell.revealWindow) {
            shell.revealWindow.destroy()
            shell.revealWindow = null
        }
        if (d && d.open)
            shell.revealWindow = shell.revealComponent.createObject(null, { shown: d })
    }
    property var lookWindow: null
    property Component lookComponent: Component {
        ProfileLook {}
    }
    function showLook(d) {
        if (d && d.open) {
            if (shell.lookWindow)
                shell.lookWindow.look = d
            else
                shell.lookWindow = shell.lookComponent.createObject(null, { look: d })
        } else if (shell.lookWindow) {
            shell.lookWindow.destroy()
            shell.lookWindow = null
        }
    }
    property Timer minuteTimer: Timer {
        interval: 60000
        repeat: true
        running: shell.session.state === "active"
        triggeredOnStart: true
        onTriggered: shell.updateMinutes()
    }

    property QtObject notchPreview: Platform.overrides.preview === "notch" && OverlayHost.shots.length > 0 ? previewComponent.createObject(shell, { notch: shell.notch }) : null
    property Component previewComponent: Component {
        NotchPreview {}
    }
    property QtObject vaultPreview: Platform.overrides.preview === "vault" && OverlayHost.shots.length > 0 ? vaultPreviewComponent.createObject(shell) : null
    property Component vaultPreviewComponent: Component {
        VaultPreview {}
    }
    property QtObject lookPreview: Platform.overrides.preview === "look" && OverlayHost.shots.length > 0 ? lookPreviewComponent.createObject(shell) : null
    property Component lookPreviewComponent: Component {
        LookPreview {}
    }

    property Timer startTimer: Timer {
        interval: 1500
        running: Daemon.status === "stopped" && !Platform.overrides.preview
        onTriggered: {
            if (Date.now() - shell.lastStart > 30000) {
                shell.lastStart = Date.now()
                Daemon.startDaemon()
            }
        }
    }

    property Connections daemonWatch: Connections {
        target: Daemon
        function onReconnected() {
            shell.refreshRecent()
            Daemon.call("look.state", {}, (ok, r) => {
                if (ok)
                    shell.showLook(r)
            })
            Daemon.call("vault.entry", {}, (ok, r) => {
                if (ok)
                    shell.showVault(r)
            })
        }
        function onSessionChanged() {
            shell.updateMinutes()
        }
    }

    property Component moverComponent: Component {
        MoveToApplications {
            onDismissed: {
                shell.mover.destroy()
                shell.mover = null
            }
        }
    }

    property Timer setupTimer: Timer {
        interval: 1200
        running: !shell.setupOffered && !shell.mover && Daemon.connected && Settings.loaded && !shell.setupDone
        onTriggered: {
            shell.setupOffered = true
            Launcher.openWindow("wizard")
        }
    }

    property Subscription events: Subscription {
        topics: ["history.added", "numbers.state", "camera.state", "look.state", "face.state", "vault.entry", "vault.reveal"]
        onReceived: (topic, d) => {
            if (topic === "history.added") {
                shell.recent = [d].concat(shell.recent).slice(0, 5)
            } else if (topic === "numbers.state") {
                if (d.visible) {
                    if (!shell.badges)
                        shell.badges = shell.badgesComponent.createObject(null)
                    shell.badges.elements = d.elements
                } else if (shell.badges) {
                    shell.badges.destroy()
                    shell.badges = null
                }
            } else if (topic === "look.state") {
                shell.showLook(d)
            } else if (topic === "face.state") {
                shell.showFace(d)
            } else if (topic === "vault.entry") {
                shell.showVault(d)
            } else if (topic === "vault.reveal") {
                shell.showReveal(d)
            } else if (topic === "camera.state") {
                if (d.open) {
                    if (!shell.camera)
                        shell.camera = shell.cameraComponent.createObject(null)
                    if (d.frame)
                        shell.camera.frame = d.frame
                    if (d.secondsLeft !== null && d.secondsLeft !== undefined)
                        shell.camera.secondsLeft = d.secondsLeft
                } else if (shell.camera) {
                    shell.camera.destroy()
                    shell.camera = null
                }
            }
        }
    }

    property Instantiator trayHolder: Instantiator {
        active: Platform.trayAvailable
        delegate: Labs.SystemTrayIcon {
            id: tray
            visible: true
            icon.source: "image://tray/" + shell.trayState + "?ink=" + (Platform.os === "linux" || Platform.darkScheme ? "ffffff" : "1a1814") + "&accent=30d158"
            tooltip: shell.tooltip
            onActivated: reason => {
                if (reason === Labs.SystemTrayIcon.Trigger || reason === Labs.SystemTrayIcon.Context)
                    trayMenu.open()
            }
            menu: Labs.Menu {
                id: trayMenu
                Labs.MenuItem {
                    text: shell.tooltip
                    enabled: false
                }
                Labs.MenuSeparator {}
                Labs.MenuItem {
                    text: shell.muted ? qsTr("Turn the microphone on") : qsTr("Mute microphone for 15 minutes")
                    onTriggered: shell.muted ? shell.mute(false) : shell.mute(true, 15)
                }
                Labs.MenuItem {
                    visible: !shell.muted
                    text: qsTr("Mute microphone until I turn it on")
                    onTriggered: shell.mute(true, null)
                }
                Labs.MenuItem {
                    visible: shell.notch.phase !== "hidden" && shell.notch.phase !== "done"
                    text: qsTr("Stop")
                    onTriggered: shell.notch.stop()
                }
                Labs.MenuItem {
                    visible: !shell.setupDone
                    text: qsTr("Finish setting up %1").arg(shell.product)
                    onTriggered: Launcher.openWindow("wizard")
                }
                Labs.MenuItem {
                    text: qsTr("Open %1").arg(shell.product)
                    onTriggered: Launcher.openWindow("home")
                }
                Labs.Menu {
                    id: recentMenu
                    title: qsTr("Recent actions")
                    enabled: shell.recent.length > 0
                    Instantiator {
                        model: shell.recent
                        delegate: Labs.MenuItem {
                            required property var modelData
                            text: modelData.undoable && modelData.action && modelData.action.id ? qsTr("Undo: %1").arg(modelData.did) : modelData.did
                            enabled: modelData.undoable === true && modelData.action && modelData.action.id !== undefined
                            onTriggered: Daemon.call("action.undo", { id: modelData.action.id })
                        }
                        onObjectAdded: (index, object) => recentMenu.insertItem(index, object)
                        onObjectRemoved: (index, object) => recentMenu.removeItem(object)
                    }
                }
                Labs.MenuItem {
                    text: qsTr("Settings")
                    onTriggered: Launcher.openWindow("settings/voice")
                }
                Labs.Menu {
                    title: qsTr("Pause for")
                    Labs.MenuItem {
                        text: qsTr("30 minutes")
                        onTriggered: shell.mute(true, 30)
                    }
                    Labs.MenuItem {
                        text: qsTr("1 hour")
                        onTriggered: shell.mute(true, 60)
                    }
                    Labs.MenuItem {
                        text: qsTr("4 hours")
                        onTriggered: shell.mute(true, 240)
                    }
                    Labs.MenuItem {
                        enabled: shell.session.state !== "dormant"
                        text: qsTr("Go to sleep now")
                        onTriggered: Daemon.call("session.sleep", {})
                    }
                }
                Labs.MenuSeparator {}
                Labs.MenuItem {
                    text: qsTr("Quit %1").arg(shell.product)
                    onTriggered: shell.notch.askQuit()
                }
            }
        }
    }
    Component.onCompleted: {
        if (Platform.outsideApplications)
            mover = moverComponent.createObject(null)
    }
}
