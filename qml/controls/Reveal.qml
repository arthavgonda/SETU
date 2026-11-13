import QtQuick
import Setu.Theme

QtObject {
    id: reveal
    required property Item target
    property int order: 0
    Component.onCompleted: {
        if (Motion.reduced || !target)
            return
        target.opacity = 0
        target.scale = 0.965
        timer.start()
    }
    property Timer timer: Timer {
        interval: 40 + reveal.order * 70
        onTriggered: reveal.anim.start()
    }
    property ParallelAnimation anim: ParallelAnimation {
        NumberAnimation { target: reveal.target; property: "opacity"; to: 1; duration: 280; easing.type: Easing.OutCubic }
        NumberAnimation { target: reveal.target; property: "scale"; to: 1; duration: 560; easing.type: Easing.OutBack; easing.overshoot: 1.5 }
    }
}
