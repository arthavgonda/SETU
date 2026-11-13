import QtQuick
import QtQuick.Layouts
import Setu.Core

Switch {
    property string key
    Layout.fillWidth: true
    checked: Settings.values[key] === true
    onToggled: Settings.set(key, checked)
}
