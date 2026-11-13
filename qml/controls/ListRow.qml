import QtQuick
import QtQuick.Templates as T
import QtQuick.Layouts
import Setu.Theme

T.ItemDelegate {
    id: row
    property string title
    property string secondary
    property string iconName
    property color iconColor: Theme.textSecondary
    property bool mono: false
    property bool selectedRow: false
    property bool interactive: true
    default property alias trailing: trailer.data
    implicitHeight: Math.max(Theme.rowHeight, body.implicitHeight + Theme.s2 * 2)
    implicitWidth: 320
    leftPadding: Theme.s3
    rightPadding: Theme.s3
    hoverEnabled: interactive
    focusPolicy: interactive ? Qt.StrongFocus : Qt.NoFocus
    Accessible.role: Accessible.ListItem
    Accessible.name: secondary.length > 0 ? title + ", " + secondary : title
    Accessible.selected: selectedRow
    Accessible.onPressAction: row.clicked()
    Keys.onReturnPressed: row.clicked()
    Keys.onEnterPressed: row.clicked()

    contentItem: RowLayout {
        id: body
        spacing: Theme.s3
        Icon {
            name: row.iconName
            color: row.iconColor
            Layout.alignment: Qt.AlignVCenter
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 0
            Label {
                Layout.fillWidth: true
                text: row.title
                role: row.mono ? "mono" : "body"
                elide: Text.ElideRight
                maximumLineCount: 2
                font.weight: row.selectedRow ? Font.DemiBold : Font.Normal
            }
            Label {
                Layout.fillWidth: true
                visible: row.secondary.length > 0
                text: row.secondary
                role: "small"
                color: Theme.textSecondary
                elide: Text.ElideRight
                maximumLineCount: 2
            }
        }
        RowLayout {
            id: trailer
            spacing: Theme.s2
            Layout.alignment: Qt.AlignVCenter
        }
    }
    background: Rectangle {
        color: row.selectedRow ? Theme.bgSelected : row.down && row.interactive ? Theme.bgPressed : row.hovered ? Theme.bgHover : "transparent"
        Rectangle {
            visible: row.selectedRow
            width: 2
            height: parent.height
            color: Theme.accent
        }
        Rectangle {
            anchors.bottom: parent.bottom
            width: parent.width
            height: 1
            color: Theme.borderSubtle
        }
        FocusRing {
            shown: row.visualFocus
            anchors.margins: 0
        }
    }
}
