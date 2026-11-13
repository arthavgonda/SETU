pragma Singleton
import QtQuick
import Setu.Core

QtObject {
    id: theme

    readonly property string choice: Platform.overrides.theme ?? Settings.values["ui.theme"] ?? "system"
    readonly property string mode: choice === "system" ? (Platform.highContrast ? "contrast" : (Platform.darkScheme ? "dark" : "light")) : choice
    readonly property bool contrast: mode === "contrast"
    readonly property bool dark: mode === "dark" || (contrast && Platform.darkScheme)

    readonly property var palettes: ({
        "light": {
            "bgBase": "#FFFFFF", "bgRaised": "#FFFFFF", "bgSunken": "#F4F4F5", "bgHover": "#F4F4F5", "bgPressed": "#E8E8EA", "bgSelected": "#F0F0F2",
            "borderSubtle": "#E4E4E7", "borderStrong": "#85858C",
            "textPrimary": "#111113", "textSecondary": "#52525A", "textTertiary": "#68686F",
            "accent": "#0A0A0B", "accentHover": "#2A2A2E", "accentPressed": "#3D3D42", "onAccent": "#FFFFFF",
            "success": "#2F6B3C", "warning": "#8A5A00", "danger": "#B42318", "info": "#1F5F8B", "onDanger": "#FFFFFF",
            "successBg": "#ECF4EC", "warningBg": "#FBF3E2", "dangerBg": "#FCECEA", "infoBg": "#EAF2F8",
            "shadow": "#26000000", "scrim": "#73000000"
        },
        "dark": {
            "bgBase": "#000000", "bgRaised": "#0F0F11", "bgSunken": "#08080A", "bgHover": "#1A1A1D", "bgPressed": "#232327", "bgSelected": "#1C1C20",
            "borderSubtle": "#26262A", "borderStrong": "#6E6E76",
            "textPrimary": "#F4F4F5", "textSecondary": "#A9A9B1", "textTertiary": "#8E8E96",
            "accent": "#FFFFFF", "accentHover": "#E4E4E7", "accentPressed": "#C9C9CE", "onAccent": "#000000",
            "success": "#6CC184", "warning": "#E0A93B", "danger": "#F2766B", "info": "#6FB1E0", "onDanger": "#1A0907",
            "successBg": "#0F1D13", "warningBg": "#21190A", "dangerBg": "#24110E", "infoBg": "#0C1A25",
            "shadow": "#99000000", "scrim": "#B3000000"
        },
        "contrastLight": {
            "bgBase": "#FFFFFF", "bgRaised": "#FFFFFF", "bgSunken": "#FFFFFF", "bgHover": "#FFFFFF", "bgPressed": "#FFFFFF", "bgSelected": "#FFFFFF",
            "borderSubtle": "#000000", "borderStrong": "#000000",
            "textPrimary": "#000000", "textSecondary": "#000000", "textTertiary": "#000000",
            "accent": "#0000C8", "accentHover": "#0000C8", "accentPressed": "#0000C8", "onAccent": "#FFFFFF",
            "success": "#005A00", "warning": "#6B4100", "danger": "#A00000", "info": "#003F80", "onDanger": "#FFFFFF",
            "successBg": "#FFFFFF", "warningBg": "#FFFFFF", "dangerBg": "#FFFFFF", "infoBg": "#FFFFFF",
            "shadow": "#00000000", "scrim": "#B3000000"
        },
        "contrastDark": {
            "bgBase": "#000000", "bgRaised": "#000000", "bgSunken": "#000000", "bgHover": "#000000", "bgPressed": "#000000", "bgSelected": "#000000",
            "borderSubtle": "#FFFFFF", "borderStrong": "#FFFFFF",
            "textPrimary": "#FFFFFF", "textSecondary": "#FFFFFF", "textTertiary": "#FFFFFF",
            "accent": "#FFFF00", "accentHover": "#FFFF00", "accentPressed": "#FFFF00", "onAccent": "#000000",
            "success": "#3FF23F", "warning": "#FFD000", "danger": "#FF6A6A", "info": "#7FC8FF", "onDanger": "#000000",
            "successBg": "#000000", "warningBg": "#000000", "dangerBg": "#000000", "infoBg": "#000000",
            "shadow": "#00000000", "scrim": "#CC000000"
        }
    })

    readonly property var p: contrast ? (Platform.darkScheme ? palettes.contrastDark : palettes.contrastLight) : (dark ? palettes.dark : palettes.light)
    readonly property bool systemAccent: contrast && Platform.highlight.hslSaturation > 0.05

    readonly property color bgBase: p.bgBase
    readonly property color bgRaised: p.bgRaised
    readonly property color bgSunken: p.bgSunken
    readonly property color bgHover: p.bgHover
    readonly property color bgPressed: p.bgPressed
    readonly property color bgSelected: p.bgSelected
    readonly property color borderSubtle: p.borderSubtle
    readonly property color borderStrong: p.borderStrong
    readonly property color textPrimary: p.textPrimary
    readonly property color textSecondary: p.textSecondary
    readonly property color textTertiary: p.textTertiary
    readonly property color accent: systemAccent ? Platform.highlight : p.accent
    readonly property color accentHover: systemAccent ? Platform.highlight : p.accentHover
    readonly property color accentPressed: systemAccent ? Platform.highlight : p.accentPressed
    readonly property color textOnAccent: systemAccent ? Platform.highlightedText : p.onAccent
    readonly property color success: p.success
    readonly property color warning: p.warning
    readonly property color danger: p.danger
    readonly property color info: p.info
    readonly property color textOnDanger: p.onDanger
    readonly property color successBg: p.successBg
    readonly property color warningBg: p.warningBg
    readonly property color dangerBg: p.dangerBg
    readonly property color infoBg: p.infoBg
    readonly property color shadow: p.shadow
    readonly property color scrim: p.scrim

    readonly property color notchBg: "#000000"
    readonly property color notchCard: "#161618"
    readonly property color notchRaised: "#2A2A2E"
    readonly property color notchInk: "#FFFFFF"
    readonly property color notchMuted: "#A1A1A8"
    readonly property color notchFaint: "#5E5E66"
    readonly property color notchShadow: "#59000000"
    readonly property color notchTimer: "#FF9F0A"
    readonly property color notchFocus: "#FF453A"
    readonly property color notchBreak: "#30D158"
    readonly property color notchDone: "#30D158"
    readonly property color notchDanger: "#FF453A"
    readonly property color notchMusic: "#30D158"
    readonly property color notchListen: "#64D2FF"
    readonly property color notchListenDeep: "#0A84FF"
    readonly property color notchSpeak: "#FF6FB5"
    readonly property color notchSpeakDeep: "#BF5AF2"
    readonly property color lookTile: "#F2F2F4"
    readonly property color lookTileHover: "#E8E8EA"
    readonly property color lookArrow: "#99000000"
    readonly property color lookArrowHover: "#CC000000"
    readonly property color lookEdge: "#40FFFFFF"
    readonly property color lookShadeTop: "#26000000"
    readonly property color lookShadeBottom: "#33000000"
    readonly property color lookTextShadow: "#80000000"
    readonly property color glow: dark ? "#14FFFFFF" : "#0A000000"

    readonly property int border: contrast ? 2 : 1
    readonly property int focusWidth: 2
    readonly property int focusGap: 2

    readonly property int s1: 4
    readonly property int s2: 8
    readonly property int s3: 12
    readonly property int s4: 16
    readonly property int s5: 24
    readonly property int s6: 32
    readonly property int s7: 48
    readonly property int s8: 64

    readonly property int radiusControl: 4
    readonly property int radiusPanel: 8
    readonly property int radiusOverlay: 12
    readonly property int radiusRound: 999

    readonly property real textScale: Math.max(1, Math.min(2, Number(Platform.overrides.scale ?? Settings.values["ui.textScale"] ?? 1))) * Platform.osTextScale
    readonly property int controlHeight: Math.round(Math.max(36, 20 * textScale + 16))
    readonly property int primaryHeight: Math.round(Math.max(44, 20 * textScale + 24))
    readonly property int rowHeight: Math.round(Math.max(44, 38 * textScale + 8))
    readonly property int hit: 32
    readonly property int iconSize: Math.round(20 * Math.min(textScale, 1.5))
    readonly property int sidebarWidth: Math.round(232 * Math.min(textScale, 1.4))

    function px(v) {
        return Math.round(v * textScale)
    }

    function hex(c) {
        const s = String(c)
        return s.length === 9 ? s.slice(3) : s.slice(1)
    }

    function icon(name, color) {
        return "image://icon/" + name + "?color=" + hex(color === undefined ? textPrimary : color)
    }

    function statusColor(level) {
        switch (level) {
        case "success": return success
        case "warning": return warning
        case "danger": return danger
        default: return info
        }
    }

    function statusBg(level) {
        switch (level) {
        case "success": return successBg
        case "warning": return warningBg
        case "danger": return dangerBg
        default: return infoBg
        }
    }

    function statusIcon(level) {
        switch (level) {
        case "success": return "circle-check"
        case "warning": return "triangle-alert"
        case "danger": return "circle-x"
        default: return "info"
        }
    }
}
