import QtQuick
import QtQuick.Controls

// Menu row on the shared scale: compact height, soft highlight, theme colours.
MenuItem {
    id: mi
    font.pixelSize: Theme.fontSizeNormal
    leftPadding: Theme.sp3
    rightPadding: Theme.sp3
    palette.text: Theme.text
    palette.highlightedText: Theme.text
    palette.windowText: Theme.text
    palette.mid: Theme.textDim
    indicator: Item {
        x: 0; width: 0
        height: mi.height
        Text { x: 2; anchors.verticalCenter: parent.verticalCenter; visible: mi.checked; text: "✓"; color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }
    }
    background: Rectangle {
        implicitWidth: 200
        implicitHeight: Theme.rowH
        radius: Theme.radius
        color: mi.highlighted ? Theme.accentSoft : "transparent"
    }
}
