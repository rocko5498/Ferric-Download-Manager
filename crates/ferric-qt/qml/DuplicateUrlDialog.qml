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
            color: Theme.textDim; font.pixelSize: 11
        },
        Text {
            text: duplicateUrlDialog.duplicateUrl
            color: Theme.text; font.pixelSize: 11
            elide: Text.ElideRight; width: parent.width
        },
        Text {
            text: qsTr("Add anyway as a separate download?")
            color: Theme.textDim; font.pixelSize: 10
        }
    ]

    buttons: [
        Rectangle {
            width: 60; height: 26; radius: 3; color: Theme.bg
            Text { anchors.centerIn: parent; text: qsTr("Cancel"); color: Theme.textDim; font.pixelSize: 11 }
            MouseArea { anchors.fill: parent; onClicked: duplicateUrlDialog.close() }
        },
        Rectangle {
            width: 100; height: 26; radius: 3; color: Theme.accent
            Text { anchors.centerIn: parent; text: qsTr("Add Anyway"); color: "white"; font.pixelSize: 11 }
            MouseArea { anchors.fill: parent; onClicked: duplicateUrlDialog.onAddAnyway() }
        }
    ]
}
