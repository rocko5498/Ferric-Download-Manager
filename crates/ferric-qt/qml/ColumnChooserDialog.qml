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
        Text { text: qsTr("Downloads"); color: Theme.accent; font.pixelSize: Theme.fontSizeSmall; font.bold: true },
        Grid {
            columns: 2; columnSpacing: Theme.sp4; rowSpacing: 2
            Repeater {
                model: columnChooserDialog.downloadColumnsAll
                Row {
                    spacing: Theme.sp2; height: Theme.rowH; width: 180
                    Rectangle {
                        width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                        property bool vis: columnChooserDialog.downloadVisibleColumns.length === 0 || columnChooserDialog.downloadVisibleColumns.indexOf(modelData.title) >= 0
                        color: vis ? Theme.accent : Theme.bg
                        border.color: Theme.surfaceAlt; border.width: 1
                        Text { anchors.centerIn: parent; text: parent.vis ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                        MouseArea { anchors.fill: parent; onClicked: { if (toggleDownloadColumn) toggleDownloadColumn(modelData.title); } }
                    }
                    Text { anchors.verticalCenter: parent.verticalCenter; text: modelData.title; color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                }
            }
        }
    ]

    buttons: [
        Rectangle { width: Theme.btnM; height: Theme.controlH; radius: Theme.radius; color: Theme.surfaceAlt
            Text { anchors.centerIn: parent; text: qsTr("Close"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
            MouseArea { anchors.fill: parent; onClicked: columnChooserDialog.close() }
        }
    ]
}
