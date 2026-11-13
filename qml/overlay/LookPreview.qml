import QtQuick
import Setu.Core
import Setu.ShellHost

QtObject {
    id: preview
    property int step: -1
    readonly property var avatars: ["cat", "cat2", "charizard_POKEMON", "dog", "dog2", "ghost_COD", "gohan_DBZ", "goku_DBZ", "picolo_DBZ", "pikachu2_POKEMON", "pikachu_POKEMON", "price_COD", "vegeta_DBZ"].map(id => ({ id: id, label: id.split("_")[0].replace(/(\d+)$/, " $1").replace(/^./, c => c.toUpperCase()), set: id.split("_")[1] || "" }))
    readonly property var backgrounds: [{ id: "COD", label: qsTr("COD") }, { id: "DBZ", label: qsTr("DBZ") }, { id: "nature", label: qsTr("Nature") }, { id: "POKEMON_01", label: qsTr("Pokemon 1") }, { id: "POKEMON_02", label: qsTr("Pokemon 2") }]
    readonly property var base: ({ open: true, id: 1, browser: "Google Chrome", display: "Chrome", family: "chromium", edit: false, avatars: avatars, backgrounds: backgrounds, custom: "", name: "", finalName: "", taken: ["Anoop", "Work"], profiles: [] })
    readonly property var steps: [
        { name: "goku", look: { step: "look", avatar: "goku_DBZ", avatarLabel: "Goku", background: "DBZ" } },
        { name: "pikachu", look: { step: "look", avatar: "pikachu_POKEMON", avatarLabel: "Pikachu", background: "POKEMON_01" } },
        { name: "cat", look: { step: "look", avatar: "cat2", avatarLabel: "Cat 2", background: "nature" } },
        { name: "name", look: { step: "name", avatar: "ghost_COD", avatarLabel: "Ghost", background: "COD", name: "Anoop", finalName: "Anoop_setu" } },
        { name: "making", look: { step: "making", avatar: "ghost_COD", avatarLabel: "Ghost", background: "COD", name: "Anoop", finalName: "Anoop_setu" } },
        { name: "import", look: { step: "import", avatar: "ghost_COD", avatarLabel: "Ghost", background: "COD", name: "Anoop", finalName: "Anoop_setu", profiles: [{ dir: "Default", name: "Anoop", email: "anoop@example.com", current: true, bytes: 734003200 }, { dir: "Profile 1", name: "Work", email: "", current: false, bytes: null }] } }
    ]
    property var window: component.createObject(null, { look: Object.assign({}, base, steps[0].look) })
    property Component component: Component {
        ProfileLook {}
    }

    property Timer driver: Timer {
        interval: 1400
        repeat: true
        running: true
        onTriggered: {
            if (preview.step >= 0)
                OverlayHost.capture(preview.window, OverlayHost.shots + "/look-" + preview.steps[preview.step].name + ".png")
            preview.step += 1
            if (preview.step >= preview.steps.length) {
                running = false
                Qt.quit()
                return
            }
            preview.window.look = Object.assign({}, preview.base, preview.steps[preview.step].look)
        }
    }
}
