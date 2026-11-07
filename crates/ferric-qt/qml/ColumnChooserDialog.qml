import QtQuick

// FQT-073: Column chooser popup \u2014 extracted from main.qml
AppDialog {
    id: columnChooserDialog

    property var downloadColumnsAll: []
    property var downloadVisibleColumns: []
    property var torrentColumnsAll: []
    property var torrentVisibleColumns: []
    property var toggleDownloadColumn
    property var toggleTorrentColumn

    title: qsTr("Choose Columns")
    cardMaxHeight: 620

    contentItem: [
        Text { text: qsTr("Downloads"); color: Theme.accent; font.pixelSize: 11; font.bold: true },
        Grid {
            columns: 2; columnSpacing: 24; rowSpacing: 2
            Repeater {
                model: columnChooserDialog.downloadColumnsAll
                Row {
                    spacing: 8; height: 20; width: 180
                    Rectangle {
                        width: 14; height: 14; radius: 3
                        property bool vis: columnChooserDialog.downloadVisibleColumns.length === 0 || columnChooserDialog.downloadVisibleColumns.indexOf(modelData.title) >= 0
                        color: vis ? Theme.accent : Theme.bg
                        border.color: Theme.surfaceAlt; border.width: 1
                        Text { anchors.centerIn: parent; text: parent.vis ? "\u2713" : ""; color: "#fff"; font.pixelSize: 9 }
                        MouseArea { anchors.fill: parent; onClicked: { if (toggleDownloadColumn) toggleDownloadColumn(modelData.title); } }
                    }
                    Text { text: modelData.title; color: Theme.text; font.pixelSize: 11 }
                }
            }
        },

        Text { text: qsTr("Torrents"); color: Theme.accent; font.pixelSize: 11; font.bold: true; topPadding: 4 },
        Grid {
            columns: 2; columnSpacing: 24; rowSpacing: 2
            Repeater {
                model: columnChooserDialog.torrentColumnsAll
                Row {
                    spacing: 8; height: 20; width: 180
                    Rectangle {
                        width: 14; height: 14; radius: 3
                        property bool vis: columnChooserDialog.torrentVisibleColumns.length === 0 || columnChooserDialog.torrentVisibleColumns.indexOf(modelData.title) >= 0
                        color: vis ? Theme.accent : Theme.bg
                        border.color: Theme.surfaceAlt; border.width: 1
                        Text { anchors.centerIn: parent; text: parent.vis ? "\u2713" : ""; color: "#fff"; font.pixelSize: 9 }
                        MouseArea { anchors.fill: parent; onClicked: { if (toggleTorrentColumn) toggleTorrentColumn(modelData.title); } }
                    }
                    Text { text: modelData.title; color: Theme.text; font.pixelSize: 11 }
                }
            }
        }
    ]

    buttons: [
        Rectangle { width: 70; height: 24; radius: 3; color: Theme.surfaceAlt
            Text { anchors.centerIn: parent; text: qsTr("Close"); color: Theme.text; font.pixelSize: 11 }
            MouseArea { anchors.fill: parent; onClicked: columnChooserDialog.close() }
        }
    ]
}
