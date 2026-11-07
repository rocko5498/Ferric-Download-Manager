import QtQuick

// Clipboard offer toast \u2014 extracted from main.qml (Phase 2 split #15)
Rectangle {
    id: clipboardOfferToast

    visible: false; z: 110
    anchors { horizontalCenter: parent.horizontalCenter; bottom: parent.bottom; bottomMargin: Theme.sp5 }
    width: Math.max(320, clipOfferText.width + 120)
    height: 44; radius: Theme.radius
    color: Theme.surface
    border.color: Theme.accent; border.width: 1

    property string offerType: "download"
    property string offerUrl: ""
    property var onAdd: function() {}

    Row {
        anchors.centerIn: parent
        spacing: Theme.sp2
        Text {
            anchors.verticalCenter: parent.verticalCenter
            id: clipOfferText
            color: Theme.text; font.pixelSize: Theme.fontSizeSmall
            text: clipboardOfferToast.offerType === "torrent"
                ? "Magnet link detected \u2014 add torrent?"
                : "URL detected \u2014 add download?"
            elide: Text.ElideRight; width: 200
        }
        Rectangle {
            width: 40; height: Theme.rowH; radius: Theme.radius; color: clipOfferAddArea.containsMouse ? Theme.accentDark : Theme.accent
            Text { anchors.centerIn: parent; text: qsTr("Add"); color: "white"; font.pixelSize: Theme.fontSizeSmall }
            MouseArea { id: clipOfferAddArea; anchors.fill: parent; hoverEnabled: true; onClicked: clipboardOfferToast.onAdd() }
        }
        Rectangle {
            width: 50; height: Theme.rowH; radius: Theme.radius; color: clipOfferDismissArea.containsMouse ? Theme.border : Theme.surfaceAlt
            Text { anchors.centerIn: parent; text: qsTr("Ignore"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            MouseArea { id: clipOfferDismissArea; anchors.fill: parent; hoverEnabled: true; onClicked: clipboardOfferToast.visible = false }
        }
    }

    // Auto-dismiss after 8 seconds
    Timer { interval: 8000; running: clipboardOfferToast.visible; onTriggered: clipboardOfferToast.visible = false }
}
