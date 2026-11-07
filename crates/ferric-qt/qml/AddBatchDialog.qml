import QtQuick

// Add Batch dialog \u2014 extracted from main.qml (Phase 2 split #5)
AppDialog {
    id: addBatchDialog

    property var onAddBatch: function(text) {}
    property alias batchText: batchInput.text

    title: qsTr("Add Batch Downloads")

    function clearInputs() {
        batchInput.text = "";
    }

    contentItem: [
        Text { text: qsTr("One URL per line. Empty lines skipped."); color: Theme.textDim; font.pixelSize: 11 },

        Rectangle {
            width: parent.width; height: 160
            radius: 4; color: Theme.bg
            border.color: batchInput.activeFocus ? Theme.accent : Theme.surfaceAlt; border.width: 1
            clip: true

            TextEdit {
                id: batchInput
                anchors { fill: parent; margins: 6 }
                color: Theme.text; font.pixelSize: 12
                wrapMode: TextEdit.NoWrap
            }
        }
    ]

    buttons: [
        Rectangle {
            width: 68; height: 28; radius: 4; color: Theme.bg
            Text { anchors.centerIn: parent; text: qsTr("Cancel"); color: Theme.textDim; font.pixelSize: 12 }
            MouseArea { anchors.fill: parent; onClicked: addBatchDialog.close() }
        },
        Rectangle {
            width: 88; height: 28; radius: 4; color: Theme.accent
            Text { anchors.centerIn: parent; text: qsTr("Add URLs"); color: "#fff"; font.pixelSize: 12 }
            MouseArea {
                anchors.fill: parent
                onClicked: addBatchDialog.onAddBatch(batchInput.text)
            }
        }
    ]
}
