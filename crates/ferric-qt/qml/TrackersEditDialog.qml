import QtQuick

// Trackers edit dialog \u2014 extracted from main.qml (Phase 2 split #12)
AppDialog {
    id: trackersEditDialog

    property string initialTrackers: ""
    property var onSave: function(text) {}

    title: qsTr("Edit Trackers")

    contentItem: [
        Text { text: qsTr("One tracker URL per line"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall },

        Rectangle {
            width: parent.width; height: 160; color: Theme.bg
            border.color: trackersTextArea.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
            clip: true
            TextEdit {
                id: trackersTextArea
                anchors { fill: parent; margins: Theme.sp1 }
                color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                wrapMode: TextEdit.NoWrap
                text: trackersEditDialog.initialTrackers
            }
        }
    ]

    buttons: [
        Rectangle {
            width: Theme.btnM; height: Theme.controlH; radius: Theme.radius; color: Theme.bg
            Text { anchors.centerIn: parent; text: qsTr("Cancel"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            MouseArea { anchors.fill: parent; onClicked: trackersEditDialog.close() }
        },
        Rectangle {
            width: Theme.btnM; height: Theme.controlH; radius: Theme.radius; color: Theme.accent
            Text { anchors.centerIn: parent; text: qsTr("Save"); color: "white"; font.pixelSize: Theme.fontSizeSmall }
            MouseArea {
                anchors.fill: parent
                onClicked: trackersEditDialog.onSave(trackersTextArea.text)
            }
        }
    ]
}
