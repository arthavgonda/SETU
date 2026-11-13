import QtQuick
import QtQuick.Templates as T
import QtQuick.Layouts
import Setu.Theme

ColumnLayout {
    id: root
    property string label
    property string helper
    property string error
    property alias text: field.text
    property alias placeholder: field.placeholderText
    property alias echoMode: field.echoMode
    property alias readOnly: field.readOnly
    property alias validator: field.validator
    property alias inputMethodHints: field.inputMethodHints
    property alias maximumLength: field.maximumLength
    property alias field: field
    property bool mono: false
    property Item leading: null
    property Item trailing: null
    signal accepted
    signal edited
    spacing: Theme.s1

    function forceActiveFocus() {
        field.forceActiveFocus()
    }

    function placeAdornments() {
        if (leading) {
            leading.parent = field
            leading.x = Theme.s3
            leading.anchors.verticalCenter = field.verticalCenter
        }
        if (trailing) {
            trailing.parent = field
            trailing.anchors.right = field.right
            trailing.anchors.rightMargin = 2
            trailing.anchors.verticalCenter = field.verticalCenter
        }
    }
    onLeadingChanged: placeAdornments()
    onTrailingChanged: placeAdornments()
    Component.onCompleted: placeAdornments()

    Label {
        text: root.label
        role: "label"
        visible: text.length > 0
        Layout.fillWidth: true
    }

    T.TextField {
        id: field
        Layout.fillWidth: true
        implicitHeight: Theme.controlHeight
        leftPadding: Theme.s3 + (root.leading ? root.leading.width + Theme.s2 : 0)
        rightPadding: Theme.s3 + (root.trailing ? root.trailing.width + Theme.s1 : 0)
        color: Theme.textPrimary
        selectionColor: Theme.accent
        selectedTextColor: Theme.textOnAccent
        placeholderTextColor: Theme.textTertiary
        verticalAlignment: TextInput.AlignVCenter
        font.family: root.mono ? Type.mono : Type.sans
        font.pixelSize: (root.mono ? Type.code : Type.body).size
        selectByMouse: true
        persistentSelection: false
        Accessible.role: Accessible.EditableText
        Accessible.name: root.label.length > 0 ? root.label : field.placeholderText
        Accessible.description: root.error.length > 0 ? root.error : root.helper
        onAccepted: root.accepted()
        onTextEdited: root.edited()
        Text {
            x: field.leftPadding
            anchors.verticalCenter: parent.verticalCenter
            width: field.width - field.leftPadding - field.rightPadding
            text: field.placeholderText
            color: Theme.textTertiary
            font: field.font
            elide: Text.ElideRight
            textFormat: Text.PlainText
            renderType: Text.NativeRendering
            visible: text.length > 0 && field.text.length === 0 && field.preeditText.length === 0
            Accessible.ignored: true
        }
        background: Rectangle {
            radius: Theme.radiusControl
            color: field.enabled ? Theme.bgSunken : Theme.bgBase
            border.width: field.activeFocus ? Math.max(2, Theme.border) : Theme.border
            border.color: root.error.length > 0 ? Theme.danger : field.activeFocus ? Theme.accent : field.hovered ? Theme.textSecondary : Theme.borderStrong
        }
    }

    RowLayout {
        Layout.fillWidth: true
        spacing: Theme.s1
        visible: root.error.length > 0 || root.helper.length > 0
        Icon {
            name: "circle-alert"
            color: Theme.danger
            size: Theme.px(16)
            visible: root.error.length > 0
            Layout.alignment: Qt.AlignTop
        }
        Label {
            Layout.fillWidth: true
            role: "small"
            text: root.error.length > 0 ? root.error : root.helper
            color: root.error.length > 0 ? Theme.danger : Theme.textSecondary
        }
    }
}
