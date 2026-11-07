import QtQuick
import Qt.labs.platform as Platform

// Add Torrent / Magnet dialog \u2014 extracted from main.qml (Phase 2 split #7)
AppDialog {
    id: addTorrentDialog

    property var onProbe: function(input) {}
    property var onStage: function(saveRoot) {}
    property var probeResult: null
    property alias torrentText: torrentInput.text
    property alias saveText: savePathField.text

    title: qsTr("Add Torrent / Magnet")

    function urlToLocalPath(u) {
        var p = u.toString().replace(/^file:\/\/\//, "").replace(/^file:\/\//, "");
        if (p.length >= 3 && p.charAt(0) === "/" && p.charAt(2) === ":")
            p = p.substring(1);
        return decodeURIComponent(p);
    }

    Platform.FileDialog {
        id: torrentFileDialog
        title: qsTr("Select .torrent file")
        nameFilters: ["Torrent files (*.torrent)", "All files (*)"]
        onAccepted: {
            torrentInput.text = addTorrentDialog.urlToLocalPath(currentFile);
            addTorrentDialog.onProbe(torrentInput.text);
        }
    }

    Platform.FolderDialog {
        id: saveFolderDialog
        title: qsTr("Select save folder")
        onAccepted: savePathField.text = addTorrentDialog.urlToLocalPath(folder)
    }

    contentItem: [
        // S14: label above torrent input — prevents mis-paste into save field
        Text { text: qsTr("Torrent file or magnet URI *"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall },
        Row {
            width: parent.width
            spacing: Theme.sp1
            Rectangle {
                width: parent.width - 74; height: 28; color: Theme.bg
                border.color: torrentInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: 3
                TextInput {
                    id: torrentInput
                    anchors { left: parent.left; right: parent.right; verticalCenter: parent.verticalCenter; leftMargin: Theme.sp1; rightMargin: Theme.sp1 }
                    color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                    clip: true
                    // S14: placeholder makes field purpose unambiguous
                    Text { visible: parent.text.length === 0; anchors.verticalCenter: parent.verticalCenter
                        text: qsTr("Paste magnet or path to .torrent"); color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal }
                }
            }
            Rectangle {
                width: 68; height: 28; radius: 3; color: Theme.surfaceAlt
                Text { anchors.centerIn: parent; text: qsTr("Browse\u2026"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { anchors.fill: parent; onClicked: torrentFileDialog.open() }
            }
        },

        Row {
            width: parent.width
            spacing: Theme.sp1
            FormField {
                id: savePathField
                label: qsTr("Save folder (optional)")
                placeholder: qsTr("Leave blank for default download directory")
                fieldWidth: parent.width - 74
            }
            Rectangle {
                width: 68; height: 28; radius: 3; color: Theme.surfaceAlt
                anchors.bottom: parent.bottom
                Text { anchors.centerIn: parent; text: qsTr("Browse…"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { anchors.fill: parent; onClicked: saveFolderDialog.open() }
            }
        },

        Rectangle {
            width: parent.width; height: 90; radius: 4; color: Theme.bg
            visible: addTorrentDialog.probeResult !== null

            Column {
                anchors { fill: parent; margins: Theme.sp2 }
                spacing: 2
                Text { text: addTorrentDialog.probeResult ? (addTorrentDialog.probeResult.name || "(unnamed)") : ""; color: Theme.text; font.pixelSize: Theme.fontSizeNormal; elide: Text.ElideRight; width: parent.width }
                Text {
                    color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
                    text: addTorrentDialog.probeResult
                        ? ((addTorrentDialog.probeResult.file_count || 0) + " file(s), " +
                           ((addTorrentDialog.probeResult.total_size || 0) / (1024*1024)).toFixed(1) + " MB" +
                           (addTorrentDialog.probeResult.is_private ? " \u00b7 private" : ""))
                        : ""
                }
                Text {
                    visible: addTorrentDialog.probeResult && addTorrentDialog.probeResult.warnings && addTorrentDialog.probeResult.warnings.length > 0
                    color: Theme.warning; font.pixelSize: Theme.fontSizeSmall; wrapMode: Text.Wrap; width: parent.width
                    text: addTorrentDialog.probeResult && addTorrentDialog.probeResult.warnings ? addTorrentDialog.probeResult.warnings.join(" \u00b7 ") : ""
                }
            }
        }
    ]

    buttons: [
        Rectangle {
            width: Theme.btnM; height: Theme.controlH; radius: Theme.radius; color: Theme.bg
            Text { anchors.centerIn: parent; text: qsTr("Cancel"); color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal }
            MouseArea { anchors.fill: parent; onClicked: addTorrentDialog.close() }
        },
        Rectangle {
            width: Theme.btnM; height: Theme.controlH; radius: Theme.radius; color: Theme.surfaceAlt
            Text { anchors.centerIn: parent; text: qsTr("Probe"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
            MouseArea { anchors.fill: parent; onClicked: addTorrentDialog.onProbe(torrentInput.text) }
        },
        Rectangle {
            width: Theme.btnM; height: Theme.controlH; radius: Theme.radius
            property bool stageable: addTorrentDialog.probeResult !== null && addTorrentDialog.probeResult.supported === true
            color: stageable ? Theme.accent : Theme.surfaceAlt
            border.color: Theme.border; border.width: stageable ? 0 : 1
            opacity: stageable ? 1.0 : 0.55
            Text { anchors.centerIn: parent; text: qsTr("Stage"); color: parent.stageable ? "#fff" : Theme.textDim; font.pixelSize: Theme.fontSizeNormal }
            MouseArea {
                anchors.fill: parent
                enabled: addTorrentDialog.probeResult && addTorrentDialog.probeResult.supported
                onClicked: addTorrentDialog.onStage(savePathField.text)
            }
        }
    ]
}
