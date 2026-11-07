import QtQuick

// Duplicate URL dialog \u2014 extracted from main.qml (Phase 2 split #14)
AppDialog {
    id: duplicateUrlDialog

    property string duplicateUrl: ""
    property var onAddAnyway: function() {}

    title: qsTr("Duplicate URL")

    contentItem: [
        Text {
            text: qsTr("This URL is already in your download list:")
            color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
        },
        Text {
            text: duplicateUrlDialog.duplicateUrl
            color: Theme.text; font.pixelSize: Theme.fontSizeSmall
            elide: Text.ElideRight; width: parent.width
        },
        Text {
            text: qsTr("Add anyway as a separate download?")
            color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
        }
    ]

    buttons: [
        Rectangle {
            width: Theme.btnM; height: Theme.controlH; radius: Theme.radius; color: Theme.bg
            Text { anchors.centerIn: parent; text: qsTr("Cancel"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            MouseArea { anchors.fill: parent; onClicked: duplicateUrlDialog.close() }
        },
        Rectangle {
            width: Theme.btnL; height: Theme.controlH; radius: Theme.radius; color: Theme.accent
            Text { anchors.centerIn: parent; text: qsTr("Add Anyway"); color: "white"; font.pixelSize: Theme.fontSizeSmall }
            MouseArea { anchors.fill: parent; onClicked: duplicateUrlDialog.onAddAnyway() }
        }
    ]
}
