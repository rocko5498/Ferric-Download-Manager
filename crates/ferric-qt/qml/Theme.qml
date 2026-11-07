pragma Singleton
import QtQuick

// One design system for the whole interface. Nothing in the QML files should hold its own pixel numbers:
// spacing, type sizes and the sizes of the main parts all come from the scales below.
//   Spacing: Fibonacci steps 5, 8, 13, 21, 34, 55 (each is the sum of the two before it, ratio ~ phi).
//   Type:    11, 13, 16, 21: 13 is the body size, the others are one step (about 1.27 = sqrt(phi)) apart.
//   Parts:   built from the 13-pixel unit (rows 2 x 13) and the Fibonacci numbers (toolbar 5 + 55 + 5).
QtObject {
    property bool dark: false

    readonly property real phi: 1.618033988749895

    // Core palette: light/dark pairs, one orange accent
    readonly property color bg: dark ? "#0d0d16" : "#f7f6f4"
    readonly property color surface: dark ? "#1a1a24" : "#ffffff"
    readonly property color surfaceAlt: dark ? "#2a2a35" : "#fdeee3"
    readonly property color border: dark ? "#3a3a4a" : "#e3dfda"
    readonly property color text: dark ? "#e0e0e0" : "#262320"
    readonly property color textDim: dark ? "#a1a1aa" : "#6b6560"
    readonly property color accent: "#e8681c"
    readonly property color accentDark: "#c4510f"
    readonly property color danger: dark ? "#f87171" : "#d64545"
    readonly property color success: dark ? "#34d399" : "#28a06c"
    readonly property color warning: dark ? "#fbbf24" : "#e8a31c"
    // soft tints for pills, selected rows and hover
    readonly property color accentSoft: dark ? "#3a2a1f" : "#fdeee3"
    readonly property color successSoft: dark ? "#16352a" : "#e4f5ec"
    readonly property color warningSoft: dark ? "#3a3013" : "#fdf1d6"
    readonly property color warningText: dark ? "#fbbf24" : "#9a6a00"
    readonly property color dangerSoft: dark ? "#3b1d1d" : "#fbe5e5"
    readonly property color rowHover: dark ? "#22222c" : "#fbf8f5"
    readonly property color shadow: "#22000000"

    // Spacing scale (Fibonacci)
    readonly property int sp1: 5
    readonly property int sp2: 8
    readonly property int sp3: 13
    readonly property int sp4: 21
    readonly property int sp5: 34
    readonly property int sp6: 55

    // Type scale
    readonly property int fontSizeSmall: 11
    readonly property int fontSizeNormal: 13
    readonly property int fontSizeMedium: 16
    readonly property int fontSizeLarge: 21

    // Parts of the window
    readonly property int unit: 13
    readonly property int rowH: 26          // list rows, table header, status bar, tab strips: 2 x unit
    readonly property int toolbarH: 65      // 5 + 55 + 5
    readonly property int toolBtnH: 55
    readonly property int toolBtnW: 89      // 55 x phi: a golden rectangle
    readonly property int toolIcon: 32
    readonly property int sidebarW: 233     // Fibonacci
    readonly property int controlH: 34      // buttons and text fields
    readonly property int listRowH: 34      // download list rows (name over host)
    readonly property int radiusCard: 8     // dialogs and cards; controls use `radius`
    readonly property int dotSize: 8        // status dots
    readonly property int btnS: 55          // short button (Save, Close): 55 x 34 is a golden rectangle
    readonly property int btnM: 89          // normal button
    readonly property int btnL: 144         // long label
    readonly property int dialogW: 610      // dialog card width (610 / 377 = phi)

    // Misc
    readonly property int radius: 5
    readonly property int animMs: 120
}
