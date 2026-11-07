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
            color: Theme.text; font.pixelSize: Theme.fontSizeNormal
        },
        Row {
            spacing: Theme.sp1
            Rectangle {
                width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                color: deleteTorrentDialog.deleteFiles ? Theme.danger : Theme.bg
                border.color: Theme.surfaceAlt; border.width: 1
                Text { anchors.centerIn: parent; text: deleteTorrentDialog.deleteFiles ? "✓" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { anchors.fill: parent; onClicked: deleteTorrentDialog.deleteFiles = !deleteTorrentDialog.deleteFiles }
            }
            Text {
                text: qsTr("Also delete files from disk")
                color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                anchors.verticalCenter: parent.verticalCenter
                MouseArea { anchors.fill: parent; onClicked: deleteTorrentDialog.deleteFiles = !deleteTorrentDialog.deleteFiles }
            }
        }
    ]

    buttons: [
        Rectangle {
            width: Theme.btnM; height: Theme.controlH; radius: Theme.radius; color: Theme.bg
            Text { anchors.centerIn: parent; text: qsTr("Cancel"); color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal }
            MouseArea {
                anchors.fill: parent
                onClicked: { deleteTorrentDialog.onCancel(); deleteTorrentDialog.close() }
            }
        },
        Rectangle {
            width: Theme.btnM; height: Theme.controlH; radius: Theme.radius; color: Theme.danger
            Text { anchors.centerIn: parent; text: qsTr("Delete"); color: "white"; font.pixelSize: Theme.fontSizeNormal }
            MouseArea {
                anchors.fill: parent
                onClicked: { deleteTorrentDialog.onConfirm(); deleteTorrentDialog.close() }
            }
        }
    ]
}
