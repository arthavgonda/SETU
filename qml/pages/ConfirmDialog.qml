import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

Dialog {
    id: dlg
    property var facts: []
    property string consequence
    property string actionLabel
    property bool danger: false
    property string word: ""
    property int timeoutMs: 0
    property bool voiceAllowed: false
    property var accept: null
    property var reject: null
    property bool answered: false
    signal confirmed(string via)
    signal cancelled(string via)

    function ask(o) {
        title = o.title
        facts = o.facts || []
        consequence = o.consequence || ""
        actionLabel = o.actionLabel || qsTr("Continue")
        danger = o.danger === true
        word = o.word || ""
        timeoutMs = o.timeoutMs || 0
        voiceAllowed = o.voiceAllowed === true
        accept = o.accept || null
        reject = o.reject || null
        typed.text = ""
        answered = false
        open()
    }

    onOpened: {
        box.start()
        if (word.length > 0)
            typed.forceActiveFocus()
    }
    onClosed: if (!answered) {
        answered = true
        cancelled("key")
        if (reject)
            reject()
    }

    TextField {
        id: typed
        Layout.fillWidth: true
        visible: dlg.word.length > 0
        label: qsTr("Type %1 to confirm").arg(dlg.word)
        mono: true
        inputMethodHints: Qt.ImhNoPredictiveText | Qt.ImhPreferUppercase
        onAccepted: if (box.actionEnabled) box.confirmed("key")
    }
    Confirmation {
        id: box
        Layout.fillWidth: true
        Layout.preferredWidth: Theme.px(480)
        title: ""
        facts: dlg.facts
        consequence: dlg.consequence
        actionLabel: dlg.actionLabel
        danger: dlg.danger
        timeoutMs: dlg.timeoutMs
        voiceAllowed: dlg.voiceAllowed
        actionEnabled: dlg.word.length === 0 || typed.text.trim() === dlg.word
        focus: dlg.word.length === 0
        onConfirmed: via => {
            dlg.answered = true
            const a = dlg.accept
            dlg.close()
            dlg.confirmed(via)
            if (a)
                a()
        }
        onCancelled: via => {
            dlg.answered = true
            const r = dlg.reject
            dlg.close()
            dlg.cancelled(via)
            if (r)
                r()
        }
        onRemainingChanged: if (dlg.visible && dlg.timeoutMs > 0 && remaining <= 0) cancelled("timeout")
    }
}
