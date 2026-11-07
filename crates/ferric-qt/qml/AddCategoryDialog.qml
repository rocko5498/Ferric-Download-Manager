import QtQuick

// Add Category dialog \u2014 extracted from main.qml (Phase 2 split #9)
AppDialog {
    id: addCategoryDialog

    property var onAdd: function(name) {}

    title: qsTr("New Category")

    contentItem: [
        Rectangle {
            width: parent.width; height: Theme.controlH; color: Theme.bg
            border.color: addCatInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
            TextInput {
                clip: true
                id: addCatInput
                anchors { fill: parent; margins: Theme.sp1 }
                color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                Keys.onReturnPressed: addCategoryDialog.onAdd(text)
                Keys.onEscapePressed: addCategoryDialog.close()
            }
        }
    ]

    buttons: [
        Rectangle {
            width: Theme.btnM; height: Theme.controlH; radius: Theme.radius; color: Theme.bg
            Text { anchors.centerIn: parent; text: qsTr("Cancel"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            MouseArea { anchors.fill: parent; onClicked: addCategoryDialog.close() }
        },
        Rectangle {
            width: Theme.btnM; height: Theme.controlH; radius: Theme.radius; color: Theme.accent
            Text { anchors.centerIn: parent; text: qsTr("Add"); color: "white"; font.pixelSize: Theme.fontSizeSmall }
            MouseArea { anchors.fill: parent; onClicked: addCategoryDialog.onAdd(addCatInput.text) }
        }
    ]
}
