import QtQuick

// Clipboard offer toast \u2014 extracted from main.qml (Phase 2 split #15)
Rectangle {
    id: clipboardOfferToast

    visible: false; z: 110
    anchors { horizontalCenter: parent.horizontalCenter; bottom: parent.bottom; bottomMargin: 40 }
    width: Math.max(320, clipOfferText.width + 120)
    height: 44; radius: 6
    color: Theme.surface
    border.color: Theme.accent; border.width: 1

    property string offerType: "download"
    property string offerUrl: ""
    property var onAdd: function() {}

    Row {
        anchors.centerIn: parent
        spacing: 10
        Text {
            id: clipOfferText
            color: Theme.text; font.pixelSize: 11
            text: clipboardOfferToast.offerType === "torrent"
                ? "Magnet link detected \u2014 add torrent?"
                : "URL detected \u2014 add download?"
            elide: Text.ElideRight; width: 200
        }
        Rectangle {
            width: 40; height: 22; radius: 3; color: clipOfferAddArea.containsMouse ? Theme.accent : Theme.accentDark
            Text { anchors.centerIn: parent; text: qsTr("Add"); color: "white"; font.pixelSize: 11 }
            MouseArea { id: clipOfferAddArea; anchors.fill: parent; hoverEnabled: true; onClicked: clipboardOfferToast.onAdd() }
        }
        Rectangle {
            width: 50; height: 22; radius: 3; color: clipOfferDismissArea.containsMouse ? Theme.border : Theme.surfaceAlt
            Text { anchors.centerIn: parent; text: qsTr("Ignore"); color: Theme.textDim; font.pixelSize: 11 }
            MouseArea { id: clipOfferDismissArea; anchors.fill: parent; hoverEnabled: true; onClicked: clipboardOfferToast.visible = false }
        }
    }

    // Auto-dismiss after 8 seconds
    Timer { interval: 8000; running: clipboardOfferToast.visible; onTriggered: clipboardOfferToast.visible = false }
}
