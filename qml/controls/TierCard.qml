import QtQuick
import QtQuick.Templates as T
import QtQuick.Layouts
import Setu.Core
import Setu.Theme

T.AbstractButton {
    id: card
    property var tier: ({})
    property bool selected: false
    property bool recommended: false
    property var catalog: []
    checkable: false
    implicitHeight: col.implicitHeight + topPadding + bottomPadding
    implicitWidth: 520
    padding: Theme.s4
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    Accessible.role: Accessible.RadioButton
    Accessible.name: (tier.name ?? "") + (recommended ? qsTr(", recommended") : "")
    Accessible.checkable: true
    Accessible.checked: selected
    Accessible.description: col.summary
    Accessible.onPressAction: card.clicked()

    function modelName(id) {
        for (const m of catalog)
            if (m.id === id)
                return m.name
        return id
    }

    contentItem: ColumnLayout {
        id: col
        readonly property string summary: [qsTr("Memory %1").arg(Platform.formatBytes(card.tier.ramBytes ?? 0)), qsTr("Disk %1").arg(Platform.formatBytes(card.tier.diskBytes ?? 0)), card.tier.speed && card.tier.speed.llmTokensPerS ? qsTr("About %1 tokens per second").arg(card.tier.speed.llmTokensPerS) : ""].filter(s => s.length > 0).join(", ")
        spacing: Theme.s2
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s3
            Rectangle {
                implicitWidth: Theme.px(18)
                implicitHeight: implicitWidth
                radius: width / 2
                color: Theme.bgSunken
                border.width: Math.max(1, Theme.border)
                border.color: card.selected ? Theme.accent : Theme.borderStrong
                Rectangle {
                    anchors.centerIn: parent
                    width: parent.width / 2
                    height: width
                    radius: width / 2
                    color: Theme.accent
                    visible: card.selected
                }
            }
            Label {
                Layout.fillWidth: true
                text: card.tier.name ?? ""
                role: "heading"
            }
            Label {
                visible: card.recommended
                text: qsTr("Recommended")
                role: "small"
                color: Theme.accent
                font.weight: Font.DemiBold
            }
            Label {
                visible: card.tier.fits === false
                text: qsTr("Needs more memory")
                role: "small"
                color: Theme.warning
            }
        }
        Label {
            Layout.fillWidth: true
            text: card.tier.reason ?? ""
            color: Theme.textSecondary
        }
        Label {
            Layout.fillWidth: true
            role: "mono"
            text: {
                const m = card.tier.models || {}
                const asr = (m.asr || []).map(card.modelName).join(" + ")
                return [qsTr("speech  %1").arg(asr), qsTr("model   %1").arg(card.modelName(m.llm)), qsTr("voice   %1").arg(card.modelName(m.tts))].join("\n")
            }
            wrapMode: Text.WrapAnywhere
        }
        Label {
            Layout.fillWidth: true
            role: "mono"
            color: Theme.textSecondary
            text: col.summary.split(", ").join("   ")
        }
    }
    background: Rectangle {
        radius: Theme.radiusPanel
        color: card.selected ? Theme.bgSelected : card.hovered ? Theme.bgHover : Theme.bgRaised
        border.width: card.selected ? 2 : Theme.border
        border.color: card.selected ? Theme.accent : Theme.borderSubtle
        FocusRing {
            shown: card.visualFocus
            radiusBase: Theme.radiusPanel
        }
    }
}
