import QtQuick
import QtQuick.Layouts
import Setu.Core
import Setu.Theme
import Ledger

ColumnLayout {
    id: form
    property string slot: "llm"
    property string provider: "openai"
    property bool custom: false
    property var current: ({})
    property bool keySaved: false
    property bool busy: false
    property bool saved: false
    property var result: null
    property var models: []
    property var voices: []
    property string model: ""
    property string voice: ""
    signal changed
    signal spoken(string text)

    readonly property string keyRef: "api." + slot
    readonly property bool local: provider === "ollama" || provider === "lmstudio"
    readonly property bool needsKey: !local
    readonly property bool needsUrl: local || custom
    readonly property var names: ({ openai: "OpenAI", anthropic: "Anthropic", gemini: "Google Gemini", elevenlabs: "ElevenLabs", ollama: "Ollama", lmstudio: "LM Studio" })
    readonly property string providerName: custom ? qsTr("the service") : (names[provider] || provider)
    readonly property var defaultUrls: ({ ollama: "http://localhost:11434/v1", lmstudio: "http://localhost:1234/v1" })
    readonly property var keyPages: ({ openai: "https://platform.openai.com/api-keys", anthropic: "https://console.anthropic.com/settings/keys", gemini: "https://aistudio.google.com/apikey", elevenlabs: "https://elevenlabs.io/app/settings/api-keys" })
    spacing: Theme.s3

    function reset() {
        result = null
        models = []
        voices = []
        saved = current.provider === provider && (current.status === "loaded" || current.status === "unloaded")
        model = current.provider === provider ? (current.modelName || "") : ""
        voice = current.provider === provider ? (current.voice || "") : ""
        baseUrl.text = current.provider === provider && current.baseUrl ? current.baseUrl : (defaultUrls[provider] || "")
        apiKey.text = ""
        keySaved = false
        SecretStore.has(keyRef, found => form.keySaved = found)
    }
    onProviderChanged: reset()
    onCustomChanged: reset()
    Component.onCompleted: reset()

    function pickModel(list) {
        if (model.length > 0 && list.indexOf(model) >= 0)
            return model
        const sorted = list.slice().sort().reverse()
        const find = re => sorted.find(m => re.test(m)) || ""
        switch (slot === "tts" ? provider + ".tts" : provider) {
        case "anthropic": return list.indexOf("claude-opus-5-5") >= 0 ? "claude-opus-5-5" : (find(/^claude-opus/) || sorted[0] || "")
        case "openai": return find(/^gpt-[0-9.]+-mini$/) || find(/^gpt-[0-9.]+$/) || find(/^gpt-/) || sorted[0] || ""
        case "gemini": return find(/^gemini-[0-9.]+-flash$/) || find(/flash(?!.*(lite|tts|image|live|audio|embed))/) || find(/^gemini/) || sorted[0] || ""
        case "openai.tts": return list.indexOf("gpt-4o-mini-tts") >= 0 ? "gpt-4o-mini-tts" : (find(/tts/) || "tts-1")
        case "elevenlabs.tts": return list.indexOf("eleven_flash_v2_5") >= 0 ? "eleven_flash_v2_5" : (find(/flash/) || find(/turbo/) || sorted[0] || "")
        default: return list[0] || ""
        }
    }
    function withKey(then) {
        const k = apiKey.text.trim()
        if (k.length === 0) {
            then(keySaved ? keyRef : "")
            return
        }
        SecretStore.store(keyRef, k, (ok, err) => {
            if (!ok) {
                form.busy = false
                form.result = { ok: false, message: qsTr("The key could not be saved to the keychain: %1").arg(err) }
                return
            }
            apiKey.text = ""
            form.keySaved = true
            then(keyRef)
        })
    }
    function params(ref) {
        let p = { slot: slot, provider: provider, keyRef: ref, modelName: model }
        if (needsUrl)
            p.baseUrl = baseUrl.text.trim()
        if (slot === "tts")
            p.voice = voice
        return p
    }
    function connect() {
        busy = true
        result = null
        saved = false
        withKey(ref => Daemon.call("models.testConnection", params(ref), (ok, r, e) => {
            if (!ok || !r.ok && (r.models || []).length === 0) {
                form.busy = false
                form.result = ok ? r : { ok: false, message: e.message }
                form.spoken(form.result.message)
                Announcer.announce(status, form.result.message, true)
                return
            }
            form.models = r.models || []
            form.voices = r.voices || []
            form.model = form.pickModel(form.models)
            if (form.slot === "tts" && (form.voice.length === 0 || !form.voices.some(v => v.id === form.voice)))
                form.voice = form.voices.length > 0 ? form.voices[0].id : ""
            form.save(ref, r)
        }))
    }
    function save(ref, tested) {
        busy = true
        Daemon.call("models.set", params(ref), (ok, r, e) => {
            form.busy = false
            if (!ok) {
                form.result = { ok: false, message: e.message }
                Announcer.announce(status, e.message, true)
                return
            }
            form.saved = true
            const what = form.slot === "tts" && form.voice.length > 0 ? (form.voices.find(v => v.id === form.voice) || { name: form.voice }).name : form.model
            form.result = { ok: true, message: qsTr("Connected to %1. Using %2.").arg(form.providerName).arg(what) + (tested && tested.latencyMs ? " " + qsTr("It answered in %1 ms.").arg(tested.latencyMs) : "") }
            form.spoken(qsTr("Connected to %1.").arg(form.providerName))
            Announcer.announce(status, form.result.message, false)
            form.changed()
        })
    }

    TextField {
        id: baseUrl
        Layout.fillWidth: true
        visible: form.needsUrl
        label: qsTr("Address")
        mono: true
        inputMethodHints: Qt.ImhUrlCharactersOnly | Qt.ImhNoAutoUppercase
        helper: form.local ? qsTr("The server must already be running on this computer or your network.") : qsTr("Any service that speaks the OpenAI API.")
    }
    TextField {
        id: apiKey
        Layout.fillWidth: true
        visible: form.needsKey
        label: qsTr("%1 API key").arg(form.providerName)
        echoMode: TextInput.Password
        mono: true
        inputMethodHints: Qt.ImhSensitiveData | Qt.ImhNoPredictiveText | Qt.ImhNoAutoUppercase
        helper: form.keySaved ? qsTr("A key is saved in the system keychain. Paste a new one to replace it.") : qsTr("Saved in the system keychain, never in %1's files.").arg(Brand.productName)
        onAccepted: form.connect()
    }
    RowLayout {
        Layout.fillWidth: true
        spacing: Theme.s2
        Button {
            variant: form.saved ? "secondary" : "primary"
            large: false
            text: form.saved ? qsTr("Connect again") : qsTr("Connect")
            busy: form.busy
            enabled: !form.busy && (!form.needsKey || form.keySaved || apiKey.text.trim().length > 0) && (!form.needsUrl || baseUrl.text.trim().length > 0)
            onClicked: form.connect()
        }
        Button {
            visible: form.needsKey && !form.custom && !!form.keyPages[form.provider]
            variant: "quiet"
            text: qsTr("Get a key")
            iconName: "external-link"
            Accessible.description: form.keyPages[form.provider] || ""
            onClicked: Platform.openUrl(form.keyPages[form.provider])
        }
        Item {
            Layout.fillWidth: true
        }
    }
    Label {
        id: status
        Layout.fillWidth: true
        visible: form.result !== null
        text: form.result ? form.result.message : ""
        color: form.result && form.result.ok ? Theme.success : Theme.danger
        Accessible.role: form.result && !form.result.ok ? Accessible.AlertMessage : Accessible.StaticText
    }
    Select {
        Layout.fillWidth: true
        visible: form.models.length > 0
        label: form.slot === "tts" ? qsTr("Speech model") : qsTr("Model")
        options: form.models.map(m => ({ value: m, text: m }))
        value: form.model
        onActivated: v => {
            form.model = v
            form.withKey(ref => form.save(ref, null))
        }
    }
    Select {
        Layout.fillWidth: true
        visible: form.slot === "tts" && form.voices.length > 0
        label: qsTr("Voice")
        options: form.voices.map(v => ({ value: v.id, text: v.name || v.id }))
        value: form.voice
        onActivated: v => {
            form.voice = v
            form.withKey(ref => form.save(ref, null))
        }
    }
    Banner {
        Layout.fillWidth: true
        visible: !form.local
        level: "warning"
        dismissible: false
        text: form.slot === "tts" ? qsTr("The text of every reply is sent to %1 to be spoken. Your voice and recordings are not.").arg(form.providerName) : qsTr("The text of your commands is sent to %1, and the screen text if you allow reading the screen. Your voice and recordings are not. %1 may charge for use.").arg(form.providerName)
    }
}
