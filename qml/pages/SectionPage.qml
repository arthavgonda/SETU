import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

FocusScope {
    id: root
    property string title
    property string description
    property var tabs: []
    property string tab: ""
    property bool searchable: false
    property var searchPages: []
    signal navigate(string page)
    signal toast(string text)
    signal search(string page, string key)
    readonly property var current: tabs.find(t => t.id === tab) || tabs[0]

    function show(id) {
        if (tabs.find(t => t.id === id))
            tab = id
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: Theme.s6
            Layout.rightMargin: Theme.s6
            Layout.topMargin: Theme.s5
            spacing: Theme.s4
            z: 10
            Label {
                Layout.fillWidth: true
                text: root.title
                role: "title"
                Accessible.role: Accessible.Heading
            }
            Item {
                visible: root.searchable
                Layout.preferredWidth: Theme.px(260)
                Layout.preferredHeight: Theme.controlHeight
                Rectangle {
                    anchors.fill: finder
                    anchors.margins: -Theme.s1
                    visible: finder.height > Theme.controlHeight + Theme.s2
                    radius: Theme.radiusPanel
                    color: Theme.bgRaised
                    border.width: Theme.border
                    border.color: Theme.borderStrong
                }
                SettingsSearch {
                    id: finder
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    anchors.bottomMargin: -(height - Theme.controlHeight)
                    pages: root.searchPages
                    compact: true
                    onChosen: (page, key) => root.search(page, key)
                }
            }
        }

        Flickable {
            Layout.fillWidth: true
            Layout.leftMargin: Theme.s5
            Layout.rightMargin: Theme.s5
            Layout.topMargin: Theme.s3
            implicitHeight: bar.implicitHeight
            contentWidth: bar.implicitWidth
            clip: true
            interactive: contentWidth > width
            boundsBehavior: Flickable.StopAtBounds
            Tabs {
                id: bar
                tabs: root.tabs
                current: root.current ? root.current.id : ""
                onActivated: id => root.tab = id
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: Theme.border
            color: Theme.borderSubtle
        }

        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            Loader {
                id: inner
                width: parent.width
                height: parent.height
                focus: true
                source: root.current ? Qt.resolvedUrl(root.current.file) : ""
                onLoaded: {
                    const p = item as Page
                    if (p) {
                        p.embedded = true
                        p.title = root.current.title
                    }
                    arrive.restart()
                }
            }
            ParallelAnimation {
                id: arrive
                NumberAnimation {
                    target: inner
                    property: "opacity"
                    from: 0
                    to: 1
                    duration: Motion.fade
                }
                NumberAnimation {
                    target: inner
                    property: "y"
                    from: Motion.reduced ? 0 : Theme.s4
                    to: 0
                    duration: Motion.spring
                    easing.type: Easing.OutBack
                    easing.overshoot: Motion.overshoot
                }
            }
        }
    }

    Connections {
        target: inner.item
        ignoreUnknownSignals: true
        function onNavigate(p) {
            root.navigate(p)
        }
        function onToast(t) {
            root.toast(t)
        }
    }
}
