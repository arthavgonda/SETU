import QtQuick
import QtQuick.Templates as T
import QtQuick.Layouts
import Setu.Core
import Setu.Theme

T.Dialog {
    id: dialog
    property bool locked: false
    property Item opener: null
    property Item initialFocus: null
    default property alias body: content.data
    property alias buttons: footerRow.data
    modal: true
    focus: true
    closePolicy: locked ? T.Popup.NoAutoClose : T.Popup.CloseOnEscape | T.Popup.CloseOnPressOutside
    anchors.centerIn: T.Overlay.overlay
    property real preferredWidth: Theme.px(560)
    readonly property real maxHeight: T.Overlay.overlay ? T.Overlay.overlay.height - Theme.s6 * 2 : Theme.px(640)
    width: Math.min(T.Overlay.overlay ? T.Overlay.overlay.width - Theme.s6 * 2 : preferredWidth, preferredWidth)
    implicitWidth: Math.max(implicitBackgroundWidth + leftInset + rightInset, contentWidth + leftPadding + rightPadding, implicitHeaderWidth, implicitFooterWidth)
    implicitHeight: Math.max(implicitBackgroundHeight + topInset + bottomInset, contentHeight + topPadding + bottomPadding + (implicitHeaderHeight > 0 ? implicitHeaderHeight + spacing : 0) + (implicitFooterHeight > 0 ? implicitFooterHeight + spacing : 0))
    height: Math.min(implicitHeight, maxHeight)
    padding: Theme.s5
    topPadding: Theme.s4
    spacing: 0

    onAboutToShow: opener = Window.window ? Window.window.activeFocusItem : null
    onOpened: (initialFocus ? initialFocus : content).forceActiveFocus()
    onClosed: if (opener) opener.forceActiveFocus()

    enter: Transition {
        ParallelAnimation {
            NumberAnimation { property: "opacity"; from: 0; to: 1; duration: Motion.fade; easing.type: Easing.OutCubic }
            NumberAnimation { property: "scale"; from: Motion.reduced ? 1 : 0.9; to: 1; duration: Motion.spring; easing.type: Easing.OutBack; easing.overshoot: Motion.overshoot }
        }
    }
    exit: Transition {
        ParallelAnimation {
            NumberAnimation { property: "opacity"; from: 1; to: 0; duration: Motion.fade; easing.type: Easing.InCubic }
            NumberAnimation { property: "scale"; from: 1; to: Motion.reduced ? 1 : 0.96; duration: Motion.fade; easing.type: Easing.InCubic }
        }
    }

    T.Overlay.modal: Rectangle {
        color: Theme.scrim
        Behavior on opacity {
            NumberAnimation { duration: Motion.fade }
        }
    }
    background: Rectangle {
        implicitWidth: dialog.preferredWidth
        radius: Theme.radiusOverlay + 4
        color: Theme.bgRaised
        border.width: Theme.border
        border.color: Theme.borderSubtle
        Rectangle {
            z: -1
            anchors.fill: parent
            anchors.topMargin: 10
            anchors.margins: -3
            radius: parent.radius + 4
            color: Theme.shadow
            opacity: 0.45
        }
    }
    header: Label {
        visible: text.length > 0
        text: dialog.title
        role: "title"
        leftPadding: Theme.s5
        rightPadding: Theme.s5
        topPadding: Theme.s5
        Accessible.role: Accessible.Heading
    }
    contentItem: Flickable {
        id: flick
        implicitWidth: content.implicitWidth
        implicitHeight: content.implicitHeight
        contentWidth: width
        contentHeight: content.implicitHeight
        clip: contentHeight > height
        boundsBehavior: Flickable.StopAtBounds
        interactive: contentHeight > height
        ColumnLayout {
            id: content
            width: flick.width
            LayoutMirroring.enabled: Platform.rightToLeft
            LayoutMirroring.childrenInherit: true
            spacing: Theme.s4
        }
        T.ScrollBar.vertical: ScrollBar {}
    }
    footer: Item {
        LayoutMirroring.enabled: Platform.rightToLeft
        LayoutMirroring.childrenInherit: true
        visible: footerRow.children.length > 0
        implicitHeight: visible ? footerRow.implicitHeight + Theme.s5 : 0
        implicitWidth: footerRow.implicitWidth + Theme.s5 * 2
        RowLayout {
            id: footerRow
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.margins: Theme.s5
            anchors.topMargin: 0
            spacing: Theme.s2
        }
    }
}
