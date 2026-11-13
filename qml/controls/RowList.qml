import QtQuick
import QtQuick.Layouts
import Setu.Theme

FocusScope {
    id: root
    property alias model: view.model
    property alias delegate: view.delegate
    property alias view: view
    property alias count: view.count
    property string label
    property bool loading: false
    property string error
    property string emptyText
    property string emptyAction
    property bool fitContent: true
    property int maxVisible: 0
    signal emptyActivated
    signal retry
    implicitHeight: view.count > 0 ? (fitContent ? Math.min(view.contentHeight, maxVisible > 0 ? maxVisible * Theme.rowHeight : view.contentHeight) : 400) : stateArea.implicitHeight
    implicitWidth: 400

    ListView {
        id: view
        anchors.fill: parent
        visible: count > 0
        clip: true
        focus: true
        keyNavigationEnabled: true
        boundsBehavior: Flickable.StopAtBounds
        reuseItems: true
        cacheBuffer: Theme.rowHeight * 10
        highlightFollowsCurrentItem: true
        activeFocusOnTab: true
        Accessible.role: Accessible.List
        Accessible.name: root.label
        ScrollBar.vertical: ScrollBar {}
        Keys.onPressed: event => {
            if (event.key === Qt.Key_Home) {
                currentIndex = 0
                event.accepted = true
            } else if (event.key === Qt.Key_End) {
                currentIndex = count - 1
                event.accepted = true
            }
        }
        onActiveFocusChanged: if (activeFocus && currentItem) currentItem.forceActiveFocus()
        onCurrentItemChanged: if (activeFocus && currentItem) currentItem.forceActiveFocus()
    }

    ColumnLayout {
        id: stateArea
        anchors.left: parent.left
        anchors.right: parent.right
        visible: view.count === 0
        LoadingState {
            visible: root.loading
            what: root.label
            Layout.fillWidth: true
        }
        ErrorState {
            visible: !root.loading && root.error.length > 0
            text: root.error
            onAction: root.retry()
            Layout.fillWidth: true
        }
        EmptyState {
            visible: !root.loading && root.error.length === 0
            text: root.emptyText
            actionText: root.emptyAction
            onAction: root.emptyActivated()
            Layout.fillWidth: true
        }
    }
}
