import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

WizardStep {
    id: step
    key: "done"
    title: qsTr("You're all set")
    subtitle: qsTr("Say “%1” any time.").arg(Brand.wakePhrase)
    summary: qsTr("Setup is finished. The user can say Hey Setu followed by a request any time, or try one of the examples. Say finish to open the dashboard.")
    primaryText: qsTr("Finish")
    primaryIcon: "check"
    property var apps: ({})
    readonly property var examples: {
        let out = []
        if (apps.browser)
            out.push(qsTr("Open %1").arg(apps.browser))
        out.push(qsTr("What time is it"))
        out.push(qsTr("Turn the volume down"))
        out.push(qsTr("Set up face unlock"))
        return out
    }
    voiceChoices: [{ id: "next", label: qsTr("finish"), phrases: ["finish", "i'm done", "open the dashboard"] }]
    function voiceChoice(id) {
        if (id === "next")
            advance()
    }
    Component.onCompleted: {
        Daemon.call("apps.list", {}, (ok, r) => {
            if (!ok)
                return
            const id = r.defaults.browser || (r.candidates.browser || [])[0]
            const app = r.items.find(x => x.id === id)
            step.apps = app ? { browser: app.name } : {}
        })
    }

    SetuFace {
        Layout.alignment: Qt.AlignHCenter
        Layout.preferredWidth: Theme.px(150)
        Layout.preferredHeight: Theme.px(72)
        mood: "happy"
    }
    StepHeader {
        Layout.fillWidth: true
        title: step.title
        subtitle: step.subtitle
    }
    Flow {
        Layout.alignment: Qt.AlignHCenter
        Layout.maximumWidth: Theme.px(520)
        spacing: Theme.s2
        Repeater {
            model: step.examples
            Pill {
                required property string modelData
                text: "“" + modelData + "”"
                Accessible.description: qsTr("Runs this command now")
                onClicked: Daemon.call("intent.run", { text: modelData }, (ok, r, e) => {
                    if (!ok)
                        step.toast(e.message)
                })
            }
        }
    }
}
