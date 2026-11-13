import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

WizardStep {
    id: step
    key: "voice"
    title: qsTr("Teach me your voice")
    subtitle: qsTr("Read this aloud three times. After that I'll only answer you.")
    summary: qsTr("Voice enrollment. A paragraph is on screen. The user reads it aloud three times, and the circles fill as each reading finishes. Then Setu only responds to their voice.")
    screenState: ({ readings: panel.count, of: 3, done: panel.done })
    spoken: qsTr("Please read the paragraph on the screen out loud, three times.")
    showPrimary: panel.done
    canContinue: panel.done
    skippable: !panel.done
    skipText: qsTr("Skip")
    voiceChoices: [
        { id: "new", label: qsTr("new paragraph"), hint: "different paragraph, other text, new text", phrases: ["new paragraph", "another paragraph", "change the text"] },
        { id: "skip", label: qsTr("skip"), hint: "skip, later, not now", phrases: ["skip this", "skip for now"] }
    ]
    function voiceChoice(id) {
        if (id === "new")
            panel.start()
        else if (id === "skip")
            advance()
    }
    Component.onCompleted: Qt.callLater(panel.start)

    StepHeader {
        Layout.fillWidth: true
        title: step.title
        subtitle: step.subtitle
    }
    EnrollPanel {
        id: panel
        Layout.fillWidth: true
        onToast: text => step.toast(text)
        onFinished: step.advance()
    }
}
