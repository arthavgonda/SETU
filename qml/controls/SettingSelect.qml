import QtQuick
import QtQuick.Layouts
import Setu.Core

Select {
    property string key
    Layout.fillWidth: true
    value: Settings.values[key]
    onActivated: v => Settings.set(key, v)
}
