import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

Rectangle {
    id: notice
    property string what: ""
    property string detail: ""
    property string actionText: qsTr("Use an online model")
    signal action
    signal dismissed

    implicitHeight: row.implicitHeight + Theme.s4 * 2
    radius: Theme.radiusPanel
    color: Theme.warningBg
    border.width: Theme.border
    border.color: Theme.warning
    Accessible.role: Accessible.AlertMessage
    Accessible.name: heading.text + " " + body.text

    onVisibleChanged: if (visible) {
        anim.restart()
        Announcer.announce(notice, Accessible.name, true)
    }

    RowLayout {
        id: row
        anchors.fill: parent
        anchors.margins: Theme.s4
        spacing: Theme.s4
        LottieAnimation {
            id: anim
            Layout.preferredWidth: Theme.px(132)
            Layout.preferredHeight: Theme.px(132)
            Layout.alignment: Qt.AlignTop
            source: "qrc:/lottie/oops.json"
            playing: !Motion.reduced && notice.visible
            loops: 2
            stillProgress: 0.5
            Accessible.ignored: true
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: Theme.s2
            Label {
                id: heading
                Layout.fillWidth: true
                text: notice.what.length > 0 ? qsTr("Oops, %1 is too much for this computer").arg(notice.what) : qsTr("Oops, this model is too much for this computer")
                role: "heading"
                Accessible.ignored: true
            }
            Label {
                id: body
                Layout.fillWidth: true
                text: notice.detail.length > 0 ? notice.detail : qsTr("It needs more memory than this computer has. You can set up an online model with your own API key instead.")
                Accessible.ignored: true
            }
            RowLayout {
                spacing: Theme.s2
                Button {
                    variant: "primary"
                    large: false
                    text: notice.actionText
                    onClicked: notice.action()
                }
                Button {
                    variant: "quiet"
                    text: qsTr("Choose another")
                    onClicked: notice.dismissed()
                }
            }
        }
    }
}
