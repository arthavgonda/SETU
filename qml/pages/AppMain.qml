import QtQuick
import Setu.Core

QtObject {
    id: app
    required property var router
    readonly property bool setupDone: Settings.values["setup.completed"] === true
    readonly property bool wantWizard: router.page === "wizard" || (Settings.loaded && !setupDone && router.page !== "settings/about")
    property var window: null

    function open() {
        const kind = wantWizard ? wizardComponent : mainComponent
        if (window && window.kind === (wantWizard ? "wizard" : "main")) {
            if (!wantWizard)
                window.go(router.page)
            window.raise()
            window.requestActivate()
            return
        }
        const old = window
        window = kind.createObject(null, { initialPage: router.page })
        if (old)
            old.destroy()
    }

    property Component mainComponent: Component {
        MainWindow {
            onWizardRequested: app.router.request("wizard")
        }
    }
    property Component wizardComponent: Component {
        WizardWindow {
            onFinished: app.router.request("home")
        }
    }

    property Connections routing: Connections {
        target: app.router
        function onRaiseRequested() {
            app.open()
        }
    }
    property Connections loading: Connections {
        target: Settings
        function onValuesChanged() {
            if (!app.window && Settings.loaded)
                app.open()
        }
    }
    property Subscription commands: Subscription {
        topics: ["ui.command"]
        onReceived: (topic, d) => {
            if (d.command === "close") {
                Qt.quit()
            } else if (d.command === "raise") {
                if (d.page)
                    app.router.request(d.page)
                else
                    app.open()
            }
        }
    }
    property Timer fallback: Timer {
        interval: 600
        running: true
        onTriggered: if (!app.window) app.open()
    }
}
