import QtQuick
import QtQuick.Controls

// Torrents table \u2014 extracted from main.qml (Phase 2 split #20)
Flickable {
    id: torrentHFlick

    anchors.fill: parent
    anchors.margins: Theme.sp2
    contentWidth: Math.max(width, tableWidth)
    contentHeight: height
    clip: true
    boundsBehavior: Flickable.StopAtBounds
    flickableDirection: Flickable.HorizontalFlick
    interactive: contentWidth > width

    property var torrentModel: null
    property var columns: []
    property var columnsAll: []
    property real tableWidth: 0
    property var selectedTorrents: []
    property string tagFilter: ""
    property string statusFilter: ""
    property var onToggleSelection: function(id, status, additive) {}
    property var onSetColWidth: function(index, width, finalize) {}
    property var onDeleteTorrent: function(id) {}
    property var onSetLocation: null
    property var bus: null
    property var requestBus: null
    property string lastError: ""
    property bool lastOk: true

    Column {
        width: torrentHFlick.tableWidth
        height: torrentHFlick.height

        Row {
            id: torrentHeaderRow
            width: parent.width
            height: 22
            Text { width: 28; text: qsTr("#"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            Repeater {
                model: torrentHFlick.columnsAll.length
                Item {
                    width: (index < torrentHFlick.columns.length && torrentHFlick.columns[index])
                           ? torrentHFlick.columns[index].width : 0
                    height: 22
                    clip: true
                    visible: width > 0
                    Text {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        anchors.rightMargin: Theme.sp2
                        text: torrentHFlick.columnsAll[index].title
                        color: Theme.textDim
                        font.pixelSize: Theme.fontSizeSmall
                        elide: Text.ElideRight
                    }
                    Rectangle {
                        anchors.right: parent.right
                        anchors.top: parent.top
                        anchors.bottom: parent.bottom
                        width: 1
                        color: resizeHandle.containsMouse || resizeHandle.pressed ? Theme.accent : Theme.surfaceAlt
                    }
                    MouseArea {
                        id: resizeHandle
                        anchors.right: parent.right
                        anchors.top: parent.top
                        anchors.bottom: parent.bottom
                        width: 8
                        hoverEnabled: true
                        cursorShape: Qt.SplitHCursor
                        preventStealing: true
                        property real pressGlobalX: 0
                        property real pressW: 0
                        onPressed: function(mouse) {
                            var g = mapToItem(torrentHeaderRow, mouse.x, mouse.y)
                            pressGlobalX = g.x
                            pressW = torrentHFlick.columns[index].width
                        }
                        onPositionChanged: function(mouse) {
                            if (!pressed) return
                            var g = mapToItem(torrentHeaderRow, mouse.x, mouse.y)
                            torrentHFlick.onSetColWidth(index, pressW + (g.x - pressGlobalX), false)
                        }
                        onReleased: function(mouse) {
                            var g = mapToItem(torrentHeaderRow, mouse.x, mouse.y)
                            torrentHFlick.onSetColWidth(index, pressW + (g.x - pressGlobalX), true)
                        }
                    }
                }
            }
        }

        ListView {
            id: torrentListView
            width: parent.width
            height: Math.max(0, parent.height - torrentHeaderRow.height)
            clip: true
            cacheBuffer: 200
            model: torrentHFlick.torrentModel
            boundsBehavior: Flickable.StopAtBounds
            flickableDirection: Flickable.VerticalFlick
            interactive: true
            delegate: Item {
                width: torrentListView.width
                property string rowTags: (model.tags === undefined || model.tags === null) ? "" : ("" + model.tags)
                property bool matchesTag: torrentHFlick.tagFilter === "" || rowTags.indexOf(torrentHFlick.tagFilter) !== -1
                property bool matchesStatus: torrentHFlick.statusFilter === "" || (model.status || "") === torrentHFlick.statusFilter
                property real rowPct: {
                    var p = Number(model.progressPct)
                    if (isNaN(p) || p < 0) return 0
                    if (p > 1) return 1
                    return p
                }
                height: (!matchesTag || !matchesStatus) ? 0 : 24
                visible: matchesTag && matchesStatus
                Rectangle {
                    anchors.fill: parent
                    color: torrentHFlick.selectedTorrents.some(function(s) { return s.id === model.id; }) ? Theme.surfaceAlt : "transparent"
                }
                Row {
                    anchors.fill: parent
                    spacing: 0
                    Text { anchors.verticalCenter: parent.verticalCenter; width: 28; color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal; text: (index + 1) }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: torrentHFlick.columns[0] ? torrentHFlick.columns[0].width : 150; elide: Text.ElideRight; color: Theme.text; font.pixelSize: Theme.fontSizeNormal; text: model.name || "" }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: torrentHFlick.columns[1] ? torrentHFlick.columns[1].width : 80; elide: Text.ElideRight; color: Theme.text; font.pixelSize: Theme.fontSizeNormal; text: model.done || "" }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: torrentHFlick.columns[2] ? torrentHFlick.columns[2].width : 80; elide: Text.ElideRight; color: Theme.text; font.pixelSize: Theme.fontSizeNormal; text: model.size || "" }
                    Item {
                        width: torrentHFlick.columns[3] ? torrentHFlick.columns[3].width : 100
                        height: parent.height
                        visible: width > 0
                        Rectangle {
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            anchors.leftMargin: 2
                            anchors.rightMargin: 2
                            height: 14
                            radius: Theme.radius
                            color: Theme.bg
                            border.color: Theme.surfaceAlt
                            border.width: 1
                            Rectangle {
                                anchors.left: parent.left
                                anchors.top: parent.top
                                anchors.bottom: parent.bottom
                                anchors.margins: 1
                                width: rowPct * Math.max(0, parent.width - 2)
                                radius: 1
                                color: model.status === "Completed" ? Theme.success
                                     : model.status === "Downloading" ? Theme.accent
                                     : model.status === "Checking" ? Theme.accent
                                     : model.status === "Paused" ? Theme.warning
                                     : model.status === "Error" ? Theme.danger
                                     : Theme.textDim
                            }
                            Text {
                                anchors.centerIn: parent
                                text: model.progress || ""
                                color: Theme.text
                                font.pixelSize: Theme.fontSizeSmall
                            }
                        }
                    }
                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        width: torrentHFlick.columns[4] ? torrentHFlick.columns[4].width : 80
                        elide: Text.ElideRight
                        color: model.status === "Downloading" ? Theme.success
                             : model.status === "Checking" ? Theme.accent
                             : model.status === "Error" ? Theme.danger
                             : model.status === "Paused" ? Theme.warning
                             : model.status === "Completed" ? Theme.success
                             : Theme.textDim
                        font.pixelSize: Theme.fontSizeNormal
                        text: model.status || ""
                    }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: torrentHFlick.columns[5] ? torrentHFlick.columns[5].width : 50; elide: Text.ElideRight; color: Theme.text; font.pixelSize: Theme.fontSizeNormal; text: model.seeds || "-" }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: torrentHFlick.columns[6] ? torrentHFlick.columns[6].width : 50; elide: Text.ElideRight; color: Theme.text; font.pixelSize: Theme.fontSizeNormal; text: model.peers || "0" }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: torrentHFlick.columns[7] ? torrentHFlick.columns[7].width : 80; elide: Text.ElideRight; color: Theme.text; font.pixelSize: Theme.fontSizeNormal; text: model.speed || "-" }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: torrentHFlick.columns[8] ? torrentHFlick.columns[8].width : 80; elide: Text.ElideRight; color: Theme.text; font.pixelSize: Theme.fontSizeNormal; text: model.upSpeed || "-" }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: torrentHFlick.columns[9] ? torrentHFlick.columns[9].width : 60; elide: Text.ElideRight; color: Theme.text; font.pixelSize: Theme.fontSizeNormal; text: model.eta || "-" }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: torrentHFlick.columns[10] ? torrentHFlick.columns[10].width : 50; elide: Text.ElideRight; color: Theme.text; font.pixelSize: Theme.fontSizeNormal; text: model.ratio || "" }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: torrentHFlick.columns[11] ? torrentHFlick.columns[11].width : 80; elide: Text.ElideRight; color: Theme.text; font.pixelSize: Theme.fontSizeNormal; text: model.category || "" }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: torrentHFlick.columns[12] ? torrentHFlick.columns[12].width : 80; elide: Text.ElideRight; color: Theme.text; font.pixelSize: Theme.fontSizeNormal; text: rowTags }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: torrentHFlick.columns[13] ? torrentHFlick.columns[13].width : 80; elide: Text.ElideRight; color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal; text: model.availability || "-" }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: torrentHFlick.columns[14] ? torrentHFlick.columns[14].width : 80; elide: Text.ElideRight; color: Theme.text; font.pixelSize: Theme.fontSizeNormal; text: model.uploaded || "" }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: torrentHFlick.columns[15] ? torrentHFlick.columns[15].width : 150; elide: Text.ElideRight; color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal; text: model.savePath || "" }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: torrentHFlick.columns[16] ? torrentHFlick.columns[16].width : 120; elide: Text.ElideRight; color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal; text: model.infoHash || "" }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: torrentHFlick.columns[17] ? torrentHFlick.columns[17].width : 100; elide: Text.ElideRight; color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal; text: model.added || "" }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: torrentHFlick.columns[18] ? torrentHFlick.columns[18].width : 100; elide: Text.ElideRight; color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal; text: model.completed || "" }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: torrentHFlick.columns[19] ? torrentHFlick.columns[19].width : 60; elide: Text.ElideRight; color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal; text: model.isPrivate || "" }
                }
                MouseArea {
                    anchors.fill: parent
                    acceptedButtons: Qt.LeftButton | Qt.RightButton
                    onClicked: function(mouse) {
                        if (mouse.button === Qt.RightButton) {
                            torrentContextMenu.rowId = model.id
                            torrentContextMenu.rowName = model.name
                            torrentContextMenu.rowInfoHash = model.infoHash
                            torrentContextMenu.popup()
                        } else {
                            torrentHFlick.onToggleSelection(
                                model.id, model.status,
                                (mouse.modifiers & Qt.ControlModifier) !== 0)
                        }
                    }
                }
            }
        }
    }

    // Empty/error state
    Text {
        anchors.centerIn: parent
        width: parent.width - 48
        z: 2
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.WordWrap
        font.pixelSize: Theme.fontSizeNormal
        visible: !torrentHFlick.lastOk && torrentHFlick.lastError !== ""
        color: Theme.danger
        text: qsTr("Cannot load torrents") + "\n" + torrentHFlick.lastError
    }

    Text {
        anchors.centerIn: parent
        width: parent.width - 48
        z: 2
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.WordWrap
        font.pixelSize: Theme.fontSizeNormal
        visible: torrentHFlick.torrentModel && torrentHFlick.torrentModel.count === 0 && torrentHFlick.lastOk
        color: Theme.textDim
        text: qsTr("No torrents yet \u2014 Add magnet or .torrent to stage")
    }

    ThemedMenu {
        id: torrentContextMenu
        property string rowId: ""
        property string rowName: ""
        property string rowInfoHash: ""
        ThemedMenuItem {
            text: qsTr("Pause/Resume")
            enabled: torrentHFlick.bus !== null
            onTriggered: {
                if (!torrentHFlick.requestBus) return
                torrentHFlick.requestBus("get_torrent", { id: torrentContextMenu.rowId }, function(result) {
                    var st = ""
                    try { st = JSON.parse(result).status || "" } catch(e) {}
                    torrentHFlick.requestBus("torrent_action", {
                        id: torrentContextMenu.rowId,
                        action: st === "Paused" ? "resume" : "pause"
                    }, function() {})
                })
            }
        }
        ThemedMenuItem {
            text: qsTr("Recheck")
            enabled: torrentHFlick.bus !== null
            onTriggered: { if (torrentHFlick.requestBus) torrentHFlick.requestBus("torrent_action", { id: torrentContextMenu.rowId, action: "recheck" }, function() {}) }
        }
        ThemedMenuItem {
            text: qsTr("Reannounce")
            enabled: torrentHFlick.bus !== null
            onTriggered: { if (torrentHFlick.requestBus) torrentHFlick.requestBus("torrent_action", { id: torrentContextMenu.rowId, action: "reannounce" }, function() {}) }
        }
        ThemedMenuSeparator {}
        ThemedMenuItem {
            text: qsTr("Copy Name")
            onTriggered: {
                clipboardInput.text = torrentContextMenu.rowName
                clipboardInput.selectAll(); clipboardInput.copy()
            }
        }
        ThemedMenuItem {
            text: qsTr("Copy Hash")
            onTriggered: {
                clipboardInput.text = torrentContextMenu.rowInfoHash
                clipboardInput.selectAll(); clipboardInput.copy()
            }
        }
        ThemedMenuItem {
            text: qsTr("Copy Magnet")
            onTriggered: {
                clipboardInput.text = "magnet:?xt=urn:btih:" + torrentContextMenu.rowInfoHash
                clipboardInput.selectAll(); clipboardInput.copy()
            }
        }
        ThemedMenuSeparator {}
        ThemedMenuItem {
            text: qsTr("Set Location")
            enabled: torrentHFlick.bus !== null
            onTriggered: {
                torrentHFlick.onSetLocation && torrentHFlick.onSetLocation(torrentContextMenu.rowId)
            }
        }
        ThemedMenuSeparator {}
        ThemedMenu {
            title: qsTr("Queue")
            ThemedMenuItem {
                text: qsTr("Move Up")
                enabled: torrentHFlick.bus !== null
                onTriggered: { if (torrentHFlick.requestBus) torrentHFlick.requestBus("set_torrent_queue", { id: torrentContextMenu.rowId, position: "up" }, function() {}) }
            }
            ThemedMenuItem {
                text: qsTr("Move Down")
                enabled: torrentHFlick.bus !== null
                onTriggered: { if (torrentHFlick.requestBus) torrentHFlick.requestBus("set_torrent_queue", { id: torrentContextMenu.rowId, position: "down" }, function() {}) }
            }
            ThemedMenuItem {
                text: qsTr("Move to Top")
                enabled: torrentHFlick.bus !== null
                onTriggered: { if (torrentHFlick.requestBus) torrentHFlick.requestBus("set_torrent_queue", { id: torrentContextMenu.rowId, position: "top" }, function() {}) }
            }
            ThemedMenuItem {
                text: qsTr("Move to Bottom")
                enabled: torrentHFlick.bus !== null
                onTriggered: { if (torrentHFlick.requestBus) torrentHFlick.requestBus("set_torrent_queue", { id: torrentContextMenu.rowId, position: "bottom" }, function() {}) }
            }
        }
        ThemedMenuSeparator {}
        ThemedMenuItem {
            text: qsTr("Remove")
            onTriggered: torrentHFlick.onDeleteTorrent(torrentContextMenu.rowId)
        }
    }
}
