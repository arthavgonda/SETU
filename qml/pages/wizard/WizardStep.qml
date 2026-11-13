import QtQuick
import QtQuick.Layouts
import Setu.Theme

ColumnLayout {
    id: step
    property string key
    property string title
    property string subtitle
    property string summary: subtitle
    property string primaryText: qsTr("Continue")
    property string primaryIcon: "arrow-right"
    property bool canContinue: true
    property bool busy: false
    property bool showPrimary: true
    property bool showBack: true
    property bool skippable: false
    property string skipText: qsTr("Skip")
    property var voiceChoices: []
    property var screenState: ({})
    property string spoken: ""
    signal toast(string text)
    signal advance
    signal retreat
    signal say(string text)
    function commit() {
        return true
    }
    function voiceChoice(id) {
    }
    function leaving() {
    }
    spacing: Theme.s5

    function reveal() {
        if (Motion.reduced)
            return
        let i = 0
        for (let k = 0; k < children.length; k++) {
            const c = children[k]
            if (!c.visible || c.opacity === 0 || c.objectName === "noreveal")
                continue
            c.opacity = 0
            revealer.createObject(step, { item: c, delay: 90 + i * Motion.stagger })
            i++
        }
    }
    Component.onCompleted: Qt.callLater(reveal)

    Component {
        id: revealer
        SequentialAnimation {
            id: anim
            property Item item
            property int delay
            running: true
            PauseAnimation {
                duration: anim.delay
            }
            NumberAnimation {
                target: anim.item
                property: "opacity"
                to: 1
                duration: 320
                easing.type: Easing.OutCubic
            }
            onFinished: anim.destroy()
        }
    }
}
