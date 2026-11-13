import QtQuick

SectionPage {
    searchable: true
    tabs: [
        { id: "voice", title: qsTr("Voice"), file: "VoicePage.qml" },
        { id: "models", title: qsTr("Models"), file: "ModelsPage.qml" },
        { id: "permissions", title: qsTr("Permissions"), file: "PermissionsPage.qml" },
        { id: "privacy", title: qsTr("Privacy"), file: "PrivacyPage.qml" },
        { id: "accessibility", title: qsTr("Accessibility"), file: "AccessibilityPage.qml" },
        { id: "general", title: qsTr("General"), file: "GeneralPage.qml" },
        { id: "apps", title: qsTr("Apps"), file: "AppsPage.qml" },
        { id: "plugins", title: qsTr("Plugins"), file: "PluginsPage.qml" }
    ]
    searchPages: tabs.concat([{ id: "about", title: qsTr("About") }])
}
