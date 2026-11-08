import QtQuick
import QtQuick.Controls

// Popup menu with a soft card look; Actions become ThemedMenuItems.
Menu {
    topPadding: Theme.sp1
    bottomPadding: Theme.sp1
    font.pixelSize: Theme.fontSizeNormal
    delegate: ThemedMenuItem {}
    background: Rectangle {
        implicitWidth: 200
        implicitHeight: Theme.rowH
        color: Theme.surface
        border.color: Theme.border
        border.width: 1
        radius: Theme.radiusCard
    }
}
