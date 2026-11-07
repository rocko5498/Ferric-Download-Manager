import QtQuick

// Set Location dialog \u2014 extracted from main.qml (Phase 2 split #13)
AppDialog {
    id: setLocationDialog

    property var onApply: function(path) {}

    title: qsTr("Set Save Location")

    contentItem: [
        FormField {
            id: setLocationInput
            label: qsTr("New folder path")
            placeholder: qsTr("New folder path for this torrent's files")
            fieldWidth: parent ? parent.width : 400
        }
    ]

    buttons: [
        Rectangle {
            width: Theme.btnM; height: Theme.controlH; radius: Theme.radius; color: Theme.bg
            Text { anchors.centerIn: parent; text: qsTr("Cancel"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            MouseArea { anchors.fill: parent; onClicked: setLocationDialog.close() }
        },
        Rectangle {
            width: Theme.btnM; height: Theme.controlH; radius: Theme.radius; color: Theme.accent
            Text { anchors.centerIn: parent; text: qsTr("Apply"); color: "white"; font.pixelSize: Theme.fontSizeSmall }
            MouseArea { anchors.fill: parent; onClicked: setLocationDialog.onApply(setLocationInput.text) }
        }
    ]
}
