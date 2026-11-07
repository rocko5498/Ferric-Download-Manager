pragma Singleton
import QtQuick

QtObject {
    property bool dark: false

    // Core palette — light/dark pairs, copper accent constant
    readonly property color bg: dark ? "#0d0d16" : "#f5f5f7"
    readonly property color surface: dark ? "#1a1a24" : "#ffffff"
    readonly property color surfaceAlt: dark ? "#2a2a35" : "#e8e8ec"
    readonly property color border: dark ? "#3a3a4a" : "#d4d4dc"
    readonly property color text: dark ? "#e0e0e0" : "#1f1f28"
    readonly property color textDim: dark ? "#a1a1aa" : "#6b6b78"
    readonly property color accent: "#b45309"
    readonly property color accentDark: "#8a3f0a"
    readonly property color danger: dark ? "#f87171" : "#dc2626"
    readonly property color success: dark ? "#34d399" : "#059669"
    readonly property color warning: dark ? "#fbbf24" : "#d97706"

    // Spacing scale
    readonly property int sp1: 4
    readonly property int sp2: 8
    readonly property int sp3: 12
    readonly property int sp4: 16
    readonly property int sp5: 24

    // Font sizes
    readonly property int fontSizeSmall: 11
    readonly property int fontSizeNormal: 13
    readonly property int fontSizeLarge: 15

    // Misc
    readonly property int radius: 4
    readonly property int animMs: 120
}
