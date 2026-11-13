import QtQuick
import QtQuick.Window
import Setu.Core

Item {
    id: host
    property var current: null
    signal toast(string text)

    Subscription {
        topics: ["prompt.confirm", "prompt.closed"]
        onReceived: (topic, d) => {
            if (topic === "prompt.confirm" && d.origin === "window") {
                host.current = d
                dialog.ask({
                    title: d.title,
                    facts: d.facts,
                    consequence: d.consequence,
                    actionLabel: d.actionLabel,
                    danger: d.danger,
                    timeoutMs: d.timeoutMs || 0,
                    voiceAllowed: d.voiceAllowed
                })
            } else if (topic === "prompt.closed" && host.current && host.current.id === d.id) {
                host.current = null
                dialog.answered = true
                dialog.close()
                if (d.outcome !== "answered" && d.message)
                    host.toast(d.message)
            }
        }
    }
    ConfirmDialog {
        id: dialog
        onConfirmed: via => {
            if (host.current)
                Daemon.call("prompt.answer", { id: host.current.id, answer: "yes", via: via === "key" ? "key" : "click" })
            host.current = null
        }
        onCancelled: via => {
            if (host.current && via !== "timeout")
                Daemon.call("prompt.answer", { id: host.current.id, answer: "no", via: via === "click" ? "click" : "key" })
            host.current = null
        }
    }
}
