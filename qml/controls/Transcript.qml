import QtQuick
import Setu.Theme

TextEdit {
    id: root
    property string finalText
    property string partialText
    readonly property bool partial: finalText.length === 0
    text: partial ? partialText : finalText
    readOnly: true
    selectByMouse: true
    selectByKeyboard: true
    activeFocusOnTab: true
    wrapMode: TextEdit.Wrap
    textFormat: TextEdit.PlainText
    font.family: Type.mono
    font.pixelSize: Type.code.size
    color: partial ? Theme.textSecondary : Theme.textPrimary
    selectionColor: Theme.accent
    selectedTextColor: Theme.textOnAccent
    persistentSelection: true
    Accessible.role: Accessible.StaticText
    Accessible.name: text
}
