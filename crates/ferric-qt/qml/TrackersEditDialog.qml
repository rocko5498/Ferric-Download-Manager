import QtQuick

// Trackers edit dialog \u2014 extracted from main.qml (Phase 2 split #12)
AppDialog {
    id: trackersEditDialog

    property string initialTrackers: ""
    property var onSave: function(text) {}

    title: qsTr("Edit Trackers")

    contentItem: [
        Text { text: qsTr("One tracker URL per line"); color: Theme.textDim; font.pixelSize: 10 },

        Rectangle {
            width: parent.width; height: 160; color: Theme.bg
            border.color: trackersTextArea.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: 3
            clip: true
            TextEdit {
                id: trackersTextArea
                anchors { fill: parent; margins: 6 }
                color: Theme.text; font.pixelSize: 11
                wrapMode: TextEdit.NoWrap
                text: trackersEditDialog.initialTrackers
            }
        }
    ]

    buttons: [
        Rectangle {
            width: 60; height: 26; radius: 3; color: Theme.bg
            Text { anchors.centerIn: parent; text: qsTr("Cancel"); color: Theme.textDim; font.pixelSize: 11 }
            MouseArea { anchors.fill: parent; onClicked: trackersEditDialog.close() }
        },
        Rectangle {
            width: 60; height: 26; radius: 3; color: Theme.accent
            Text { anchors.centerIn: parent; text: qsTr("Save"); color: "white"; font.pixelSize: 11 }
            MouseArea {
                anchors.fill: parent
                onClicked: trackersEditDialog.onSave(trackersTextArea.text)
            }
        }
    ]
}
