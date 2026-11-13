import QtQuick
import Setu.Core
import Setu.ShellHost

QtObject {
    id: preview
    property int step: -1
    property var window: null
    readonly property var steps: [
        { name: "face-start", kind: "face", props: { mode: "enrol", message: "Look at the camera.", steps: {} } },
        { name: "face-half", kind: "face", props: { mode: "enrol", message: "Turn your head slowly: left, right, up and down.", steps: { center: true, left: true, up: true } } },
        { name: "face-done", kind: "face", props: { mode: "enrol", message: "All done.", steps: { center: true, left: true, right: true, up: true, down: true } } },
        { name: "face-check", kind: "face", props: { mode: "check", message: "Look at the camera.", steps: {} } },
        { name: "card-save", kind: "card", props: { entry: { open: true, id: "p1", mode: "save", title: qsTr("Google"), account: "", message: "Type the password. It stays on this computer." } } },
        { name: "card-system", kind: "card", props: { entry: { open: true, id: "p2", mode: "system", title: qsTr("System password"), account: "anoop", message: "Type the password you use to sign in to this computer.", error: "That isn't your system password. Try again." } } },
        { name: "reveal", kind: "reveal", props: { shown: { open: true, title: qsTr("Google"), account: "anoop@example.com", password: "correct-horse-battery", seconds: 10 } } }
    ]
    property Component face: Component {
        FaceSetup {}
    }
    property Component card: Component {
        VaultCard {}
    }
    property Component reveal: Component {
        VaultReveal {}
    }

    property Timer driver: Timer {
        interval: 1400
        repeat: true
        running: true
        triggeredOnStart: true
        onTriggered: {
            if (preview.step >= 0 && preview.window)
                OverlayHost.capture(preview.window, OverlayHost.shots + "/vault-" + preview.steps[preview.step].name + ".png")
            preview.step += 1
            if (preview.window) {
                preview.window.destroy()
                preview.window = null
            }
            if (preview.step >= preview.steps.length) {
                running = false
                Qt.quit()
                return
            }
            const s = preview.steps[preview.step]
            preview.window = (s.kind === "face" ? preview.face : s.kind === "card" ? preview.card : preview.reveal).createObject(null, s.props)
        }
    }
}
