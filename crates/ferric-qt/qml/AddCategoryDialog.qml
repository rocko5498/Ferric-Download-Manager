import QtQuick

// Add Category dialog \u2014 extracted from main.qml (Phase 2 split #9)
AppDialog {
    id: addCategoryDialog

    property var onAdd: function(name) {}

    title: qsTr("New Category")

    contentItem: [
        Rectangle {
            width: parent.width; height: 28; color: Theme.bg
            border.color: addCatInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: 3
            TextInput {
                clip: true
                id: addCatInput
                anchors { fill: parent; margins: 6 }
                color: Theme.text; font.pixelSize: 12
                Keys.onReturnPressed: addCategoryDialog.onAdd(text)
                Keys.onEscapePressed: addCategoryDialog.close()
            }
        }
    ]

    buttons: [
        Rectangle {
            width: 60; height: 26; radius: 3; color: Theme.bg
            Text { anchors.centerIn: parent; text: qsTr("Cancel"); color: Theme.textDim; font.pixelSize: 11 }
            MouseArea { anchors.fill: parent; onClicked: addCategoryDialog.close() }
        },
        Rectangle {
            width: 60; height: 26; radius: 3; color: Theme.accent
            Text { anchors.centerIn: parent; text: qsTr("Add"); color: "white"; font.pixelSize: 11 }
            MouseArea { anchors.fill: parent; onClicked: addCategoryDialog.onAdd(addCatInput.text) }
        }
    ]
}
