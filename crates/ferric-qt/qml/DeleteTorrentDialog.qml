import QtQuick

// Delete torrent confirm dialog \u2014 extracted from main.qml (Phase 2 split #8)
AppDialog {
    id: deleteTorrentDialog

    property int deleteCount: 0
    property bool deleteFiles: false
    property var onConfirm: function() {}
    property var onCancel: function() {}

    onVisibleChanged: if (visible) deleteFiles = false

    title: qsTr("Delete torrents")

    contentItem: [
        Text {
            text: "Delete " + deleteTorrentDialog.deleteCount + " torrent(s)?"
            color: Theme.text; font.pixelSize: 13
        },
        Row {
            spacing: 6
            Rectangle {
                width: 16; height: 16; radius: 3
                color: deleteTorrentDialog.deleteFiles ? Theme.danger : Theme.bg
                border.color: Theme.surfaceAlt; border.width: 1
                Text { anchors.centerIn: parent; text: deleteTorrentDialog.deleteFiles ? "✓" : ""; color: "#fff"; font.pixelSize: 10 }
                MouseArea { anchors.fill: parent; onClicked: deleteTorrentDialog.deleteFiles = !deleteTorrentDialog.deleteFiles }
            }
            Text {
                text: qsTr("Also delete files from disk")
                color: Theme.text; font.pixelSize: 12
                anchors.verticalCenter: parent.verticalCenter
                MouseArea { anchors.fill: parent; onClicked: deleteTorrentDialog.deleteFiles = !deleteTorrentDialog.deleteFiles }
            }
        }
    ]

    buttons: [
        Rectangle {
            width: 64; height: 28; radius: 4; color: Theme.bg
            Text { anchors.centerIn: parent; text: qsTr("Cancel"); color: Theme.textDim; font.pixelSize: 12 }
            MouseArea {
                anchors.fill: parent
                onClicked: { deleteTorrentDialog.onCancel(); deleteTorrentDialog.close() }
            }
        },
        Rectangle {
            width: 90; height: 28; radius: 4; color: Theme.danger
            Text { anchors.centerIn: parent; text: qsTr("Delete"); color: "white"; font.pixelSize: 12 }
            MouseArea {
                anchors.fill: parent
                onClicked: { deleteTorrentDialog.onConfirm(); deleteTorrentDialog.close() }
            }
        }
    ]
}
