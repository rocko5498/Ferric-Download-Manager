import QtQuick
import QtQuick.Controls as QQC
import com.ferric.app 1.0

// FQT-033: torrent detail panel \u2014 extracted from main.qml (Phase 2 split #2)
Rectangle {
    id: panel

    // Anchors + visible bound by parent instance in main.qml
    z: 10
    height: 214
    color: Theme.bg
    border.color: Theme.surfaceAlt
    border.width: 1

    property var bus
    property var requestBus
    property var hub
    property var selectedTorrents: []
    property bool selectionCanQueueMove: false
    property var torrentPeers: []
    property string torrentPeersError: ""

    // Expose general-tab payload for parent refresh/toggle helpers
    property alias tdData: torrentDetailGeneral.tdData

    // Parent-owned callbacks (dialogs stay in main.qml)
    property var refreshTorrentDetail
    property var refreshTorrentPeers
    property var doToggleTorrentFile
    property var doOpenTorrentFolder
    property var doCopyInfoHash
    property var doCopyMagnet
    property var doTorrentRecheck
    property var doTorrentReannounce
    property var doShowSetLocation
    property var openTrackersEdit
    property var openTorrentPolicy
    property var showError

    onVisibleChanged: if (visible && refreshTorrentDetail) refreshTorrentDetail()
    onSelectedTorrentsChanged: {
        if (!visible) return
        if (refreshTorrentDetail) refreshTorrentDetail()
        if (torrentDetailTab.activeTab === 3 && refreshTorrentPeers) refreshTorrentPeers()
    }

    Connections {
        target: hub
        function onUpdated() {
            if (!panel.visible) return
            if (refreshTorrentDetail) refreshTorrentDetail()
            if (torrentDetailTab.activeTab === 3 && refreshTorrentPeers) refreshTorrentPeers()
        }
    }

    Column {
        anchors.fill: parent
        spacing: 0

        // Two stacked scroll-clipped rows: tabs, then actions — they collided
        // in one 28px row the moment the window narrowed below ~1300px.
        Item {
            id: torrentDetailTab
            property int activeTab: 0
            width: parent.width
            height: 52

            Flickable {
                id: torrentTabFlick
                anchors.top: parent.top
                width: parent.width
                height: 28
                contentWidth: torrentTabsRow.width
                contentHeight: 28
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                flickableDirection: Flickable.HorizontalFlick

            Row {
                id: torrentTabsRow
                Rectangle {
                    width: 90; height: 28
                    color: torrentDetailTab.activeTab === 0 ? Theme.surface : "transparent"
                    MouseArea { anchors.fill: parent; onClicked: torrentDetailTab.activeTab = 0 }
                    Text { anchors.centerIn: parent; text: qsTr("General"); color: torrentDetailTab.activeTab === 0 ? Theme.text : Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                }
                Rectangle {
                    width: 90; height: 28
                    color: torrentDetailTab.activeTab === 1 ? Theme.surface : "transparent"
                    MouseArea { anchors.fill: parent; onClicked: torrentDetailTab.activeTab = 1 }
                    Text { anchors.centerIn: parent; text: qsTr("Files"); color: torrentDetailTab.activeTab === 1 ? Theme.text : Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                }
                Rectangle {
                    width: 90; height: 28
                    color: torrentDetailTab.activeTab === 2 ? Theme.surface : "transparent"
                    MouseArea { anchors.fill: parent; onClicked: torrentDetailTab.activeTab = 2 }
                    Text { anchors.centerIn: parent; text: qsTr("Trackers"); color: torrentDetailTab.activeTab === 2 ? Theme.text : Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                }
                Rectangle {
                    width: 90; height: 28
                    color: torrentDetailTab.activeTab === 3 ? Theme.surface : "transparent"
                    MouseArea {
                        anchors.fill: parent
                        onClicked: { torrentDetailTab.activeTab = 3; refreshTorrentPeers(); }
                    }
                    Text { anchors.centerIn: parent; text: qsTr("Peers"); color: torrentDetailTab.activeTab === 3 ? Theme.text : Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                }
                Rectangle {
                    // T2 honesty: Pieces often has no engine map \u2014 keep tab but dim + n/a label.
                    width: 100; height: 28
                    color: torrentDetailTab.activeTab === 4 ? Theme.surface : "transparent"
                    opacity: 0.75
                    MouseArea {
                        anchors.fill: parent
                        onClicked: { torrentDetailTab.activeTab = 4; }
                    }
                    Text {
                        anchors.centerIn: parent
                        text: qsTr("Pieces")
                        color: torrentDetailTab.activeTab === 4 ? Theme.textDim : Theme.textDim
                        font.pixelSize: Theme.fontSizeSmall
                    }
                }
                Rectangle {
                    width: 100; height: 28
                    color: torrentDetailTab.activeTab === 5 ? Theme.surface : "transparent"
                    MouseArea { anchors.fill: parent; onClicked: torrentDetailTab.activeTab = 5 }
                    Text { anchors.centerIn: parent; text: qsTr("HTTP Sources"); color: torrentDetailTab.activeTab === 5 ? Theme.text : Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                }
                Rectangle {
                    width: 90; height: 28
                    color: torrentDetailTab.activeTab === 6 ? Theme.surface : "transparent"
                    MouseArea { anchors.fill: parent; onClicked: torrentDetailTab.activeTab = 6 }
                    Text { anchors.centerIn: parent; text: qsTr("Speed"); color: torrentDetailTab.activeTab === 6 ? Theme.text : Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                }
            }
            }

            // FQT-034: sync button \u2014 re-fetches live stats + peers from the engine
            Flickable {
                anchors.top: torrentTabFlick.bottom
                width: parent.width
                height: 24
                contentWidth: torrentActionsRow.width + 16
                contentHeight: 24
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                flickableDirection: Flickable.HorizontalFlick

            Row {
                id: torrentActionsRow
                x: 8
                y: 2
                spacing: Theme.sp1
                Rectangle {
                    width: 60; height: 20; radius: 3; color: torOpenFolderArea.containsMouse ? Theme.surfaceAlt : Theme.surface
                    Text { anchors.centerIn: parent; text: qsTr("Folder"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea { id: torOpenFolderArea; anchors.fill: parent; hoverEnabled: true; onClicked: doOpenTorrentFolder() }
                }
                Rectangle {
                    width: 62; height: 20; radius: 3; color: torCopyHashArea.containsMouse ? Theme.surfaceAlt : Theme.surface
                    Text { anchors.centerIn: parent; text: qsTr("Copy Hash"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea { id: torCopyHashArea; anchors.fill: parent; hoverEnabled: true; onClicked: doCopyInfoHash() }
                }
                Rectangle {
                    width: 68; height: 20; radius: 3; color: torCopyMagnetArea.containsMouse ? Theme.surfaceAlt : Theme.surface
                    Text { anchors.centerIn: parent; text: qsTr("Copy Magnet"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea { id: torCopyMagnetArea; anchors.fill: parent; hoverEnabled: true; onClicked: doCopyMagnet() }
                }
                Rectangle {
                    width: 56; height: 20; radius: 3; color: torRecheckArea.containsMouse ? Theme.surfaceAlt : Theme.surface
                    Text { anchors.centerIn: parent; text: qsTr("Recheck"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea { id: torRecheckArea; anchors.fill: parent; hoverEnabled: true; onClicked: doTorrentRecheck() }
                }
                Rectangle {
                    width: 72; height: 20; radius: 3; color: torReannounceArea.containsMouse ? Theme.surfaceAlt : Theme.surface
                    Text { anchors.centerIn: parent; text: qsTr("Reannounce"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea { id: torReannounceArea; anchors.fill: parent; hoverEnabled: true; onClicked: doTorrentReannounce() }
                }
                Rectangle {
                    width: 56; height: 20; radius: 3; color: torLocArea.containsMouse ? Theme.surfaceAlt : Theme.surface
                    Text { anchors.centerIn: parent; text: qsTr("Location"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea { id: torLocArea; anchors.fill: parent; hoverEnabled: true; onClicked: doShowSetLocation() }
                }
                Rectangle {
                    width: 62; height: 20; radius: 3; color: torTrackersArea.containsMouse ? Theme.surfaceAlt : Theme.surface
                    Text { anchors.centerIn: parent; text: qsTr("Trackers"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea {
                        id: torTrackersArea
                        anchors.fill: parent; hoverEnabled: true
                        onClicked: if (openTrackersEdit) openTrackersEdit()
                    }
                }
                Rectangle {
                    width: 20; height: 20; radius: 3; color: selectionCanQueueMove ? (torUpArea.containsMouse ? Theme.surfaceAlt : Theme.surface) : Theme.bg
                    opacity: selectionCanQueueMove ? 1.0 : 0.4
                    Text { anchors.centerIn: parent; text: qsTr("\u25b2"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea { id: torUpArea; anchors.fill: parent; hoverEnabled: true; enabled: selectionCanQueueMove; onClicked: { if (selectedTorrents.length === 1 && requestBus) requestBus("set_torrent_queue", { id: selectedTorrents[0].id, position: "up" }, function() {}); } }
                }
                Rectangle {
                    width: 20; height: 20; radius: 3; color: selectionCanQueueMove ? (torDownArea.containsMouse ? Theme.surfaceAlt : Theme.surface) : Theme.bg
                    opacity: selectionCanQueueMove ? 1.0 : 0.4
                    Text { anchors.centerIn: parent; text: qsTr("\u25bc"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea { id: torDownArea; anchors.fill: parent; hoverEnabled: true; enabled: selectionCanQueueMove; onClicked: { if (selectedTorrents.length === 1 && requestBus) requestBus("set_torrent_queue", { id: selectedTorrents[0].id, position: "down" }, function() {}); } }
                }
                Rectangle {
                    width: 50; height: 20; radius: 3; color: policyArea.containsMouse ? Theme.surfaceAlt : Theme.surface
                    Text { anchors.centerIn: parent; text: qsTr("Policy"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea {
                        id: policyArea
                        anchors.fill: parent; hoverEnabled: true
                        onClicked: if (openTorrentPolicy) openTorrentPolicy()
                    }
                }
                Rectangle {
                    width: 50; height: 20; radius: 3; color: syncArea.containsMouse ? Theme.surfaceAlt : Theme.surface
                    Text { anchors.centerIn: parent; text: qsTr("Sync"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea {
                        id: syncArea
                        anchors.fill: parent; hoverEnabled: true
                        onClicked: {
                            if (!requestBus) return
                            requestBus("sync_torrent", { id: selectedTorrents[0].id }, function(result) {
                                var res = JSON.parse(result)
                                if (res.error) { showError(res.error); return; }
                                torrentDetailGeneral.tdData = res
                                if (torrentDetailTab.activeTab === 3 && refreshTorrentPeers) refreshTorrentPeers()
                            })
                        }
                    }
                }
            }
            }
        }

        Rectangle { width: parent.width; height: 1; color: Theme.surfaceAlt }

        // General tab
        Item {
            id: torrentDetailGeneral
            property var tdData: null
            width: parent.width
            height: parent.height - 53
            visible: torrentDetailTab.activeTab === 0

            Flickable {
                id: torrentGeneralFlick
                anchors.fill: parent
                contentWidth: width
                contentHeight: proxyRow.y + proxyRow.height + 12
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                QQC.ScrollBar.vertical: QQC.ScrollBar {}

            Grid {
                id: torrentGeneralGrid
                anchors { top: parent.top; left: parent.left; right: parent.right; margins: Theme.sp2 }
                height: implicitHeight
                columns: 2
                columnSpacing: Theme.sp3
                rowSpacing: Theme.sp1

                Text { text: qsTr("Name:");   color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Text { text: torrentDetailGeneral.tdData ? (torrentDetailGeneral.tdData.record.name || "-") : "-"; color: Theme.text; font.pixelSize: Theme.fontSizeSmall; elide: Text.ElideRight; width: 320 }
                Text { text: qsTr("Hash:");   color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Text { text: torrentDetailGeneral.tdData ? (torrentDetailGeneral.tdData.record.info_hash || "-") : "-"; color: Theme.text; font.pixelSize: Theme.fontSizeSmall; elide: Text.ElideRight; width: 320 }
                Text { text: qsTr("Path:");   color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Text { text: torrentDetailGeneral.tdData ? (torrentDetailGeneral.tdData.record.save_root || "-") : "-"; color: Theme.text; font.pixelSize: Theme.fontSizeSmall; elide: Text.ElideRight; width: 320 }
                Text { text: qsTr("Status:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Text { text: torrentDetailGeneral.tdData ? (torrentDetailGeneral.tdData.record.status || "-") : "-"; color: Theme.accent; font.pixelSize: Theme.fontSizeSmall }
                Text { text: qsTr("Tags:");   color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Text { text: torrentDetailGeneral.tdData ? ((torrentDetailGeneral.tdData.record.tags || []).join(", ") || "none") : "-"; color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                Text { text: qsTr("Size:");   color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Text {
                    color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                    text: {
                        if (!torrentDetailGeneral.tdData) return "-"
                        var t = torrentDetailGeneral.tdData.record.total_size || 0
                        var d = torrentDetailGeneral.tdData.record.downloaded || 0
                        return (t > 0 ? (d / (1024*1024)).toFixed(1) + " / " + (t / (1024*1024)).toFixed(1) + " MB" : "-")
                    }
                }
                Text { text: qsTr("Ratio:");  color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Text { text: torrentDetailGeneral.tdData ? (torrentDetailGeneral.tdData.record.ratio || 0).toFixed(2) : "-"; color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
            }

            // Tag assignment UI
            Row {
                id: tagRow
                anchors { top: torrentGeneralGrid.bottom; left: parent.left; right: parent.right; margins: Theme.sp2; topMargin: Theme.sp1 }
                spacing: Theme.sp1
                Text { text: qsTr("Add tag:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Rectangle {
                    width: 120; height: 20; radius: 3; color: Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                    TextInput { id: tagInput; anchors.fill: parent; anchors.margins: Theme.sp1; color: Theme.text; font.pixelSize: Theme.fontSizeSmall; clip: true }
                clip: true
                }
                Rectangle {
                    width: 40; height: 20; radius: 3; color: tagAddArea.containsMouse ? Theme.border : Theme.surfaceAlt
                    Text { anchors.centerIn: parent; text: qsTr("Add"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea { id: tagAddArea; anchors.fill: parent; hoverEnabled: true
                        onClicked: {
                            if (tagInput.text === "" || !selectedTorrents.length) return;
                            if (!requestBus) return;
                            requestBus("add_torrent_tag", { id: selectedTorrents[0].id, tag: tagInput.text }, function() {
                                tagInput.text = "";
                                refreshTorrentDetail();
                            });
                        }
                    }
                }
            }
            Flow {
                id: tagFlow
                anchors { top: tagRow.bottom; left: parent.left; right: parent.right; margins: Theme.sp2; topMargin: Theme.sp1 }
                height: visible ? implicitHeight : 0
                spacing: Theme.sp1
                visible: torrentDetailGeneral.tdData ? (torrentDetailGeneral.tdData.record && torrentDetailGeneral.tdData.record.tags && torrentDetailGeneral.tdData.record.tags.length > 0) : false
                Repeater {
                    model: torrentDetailGeneral.tdData ? (torrentDetailGeneral.tdData.record.tags || []) : []
                    Rectangle {
                        width: tagLabel.width + 20; height: 18; radius: 3; color: Theme.surfaceAlt
                        Row { anchors.centerIn: parent; spacing: Theme.sp1
                            Text { id: tagLabel; text: modelData; color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            Text { text: qsTr("\u00d7"); color: Theme.danger; font.pixelSize: Theme.fontSizeSmall
                                MouseArea { anchors.fill: parent; onClicked: {
                                    if (requestBus) requestBus("remove_torrent_tag", { id: selectedTorrents[0].id, tag: modelData }, function() {
                                        refreshTorrentDetail();
                                    });
                                }}
                            }
                        }
                    }
                }
            }

            // Proxy assignment UI
            Row {
                id: proxyRow
                anchors { top: tagFlow.bottom; left: parent.left; right: parent.right; margins: Theme.sp2; topMargin: Theme.sp1 }
                spacing: Theme.sp1
                Text { text: qsTr("Proxy:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Rectangle {
                    width: 180; height: 20; radius: 3; color: Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                    property var proxyList: []
                    Component.onCompleted: {
                        if (!requestBus) return;
                        requestBus("get_proxies", {}, function(raw) {
                            try { proxyList = JSON.parse(raw); } catch(e) {}
                        });
                    }
                    Text {
                        anchors.fill: parent; anchors.margins: Theme.sp1
                        text: {
                            if (!torrentDetailGeneral.tdData) return "\u2014";
                            var assigned = torrentDetailGeneral.tdData.record.proxy_id || "";
                            if (!assigned) return "Default (global)";
                            for (var i = 0; i < parent.proxyList.length; i++) {
                                if (parent.proxyList[i].id === assigned) return parent.proxyList[i].name || parent.proxyList[i].host;
                            }
                            return assigned;
                        }
                        color: Theme.text; font.pixelSize: Theme.fontSizeSmall; clip: true; elide: Text.ElideRight
                    }
                    MouseArea {
                        anchors.fill: parent
                        onClicked: {
                            if (!requestBus) return;
                            requestBus("get_proxies", {}, function(raw) {
                                var items = ["Default (global)"];
                                var ids = [""];
                                try {
                                    var proxies = JSON.parse(raw);
                                    for (var i = 0; i < proxies.length; i++) {
                                        items.push(proxies[i].name || proxies[i].host || proxies[i].id);
                                        ids.push(proxies[i].id);
                                    }
                                } catch(e) {}
                                proxyChooser.items = items;
                                proxyChooser.proxyIds = ids;
                                proxyChooser.visible = true;
                            });
                        }
                    }
                }
            }
            Rectangle {
                id: proxyChooser
                visible: false; z: 100
                x: 60; y: proxyRow.y + proxyRow.height
                width: 200; height: proxyColumn.height + 8; radius: 4; color: Theme.surface; border.color: Theme.border; border.width: 1
                property var items: []
                property var proxyIds: []
                Column { id: proxyColumn; anchors { left: parent.left; right: parent.right; margins: Theme.sp1 } spacing: 2
                    Repeater {
                        model: proxyChooser.items
                        Rectangle {
                            width: proxyColumn.width; height: 22; radius: 3; color: proxyMouse.containsMouse ? Theme.surfaceAlt : "transparent"
                            Text { anchors { left: parent.left; margins: Theme.sp1; verticalCenter: parent.verticalCenter } text: modelData; color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                            MouseArea { id: proxyMouse; anchors.fill: parent; hoverEnabled: true
                                onClicked: {
                                    var tid = selectedTorrents.length ? selectedTorrents[0].id : "";
                                    if (tid) bus.assign_torrent_proxy(tid, proxyChooser.proxyIds[index]);
                                    proxyChooser.visible = false;
                                    refreshTorrentDetail();
                                }
                            }
                        }
                    }
                }
            }
            }
        }

        // Files tab
        Item {
            id: torrentDetailFiles
            width: parent.width
            height: parent.height - 53
            visible: torrentDetailTab.activeTab === 1

            ListView {
                anchors { fill: parent; margins: Theme.sp2 }
                model: torrentDetailGeneral.tdData ? torrentDetailGeneral.tdData.files : []
                clip: true
                cacheBuffer: 200
                delegate: Row {
                    spacing: Theme.sp2; height: 20; width: ListView.view.width

                    Rectangle {
                        width: 14; height: 14; radius: 2
                        anchors.verticalCenter: parent.verticalCenter
                        color: modelData.selected ? Theme.accent : Theme.bg
                        border.color: Theme.surfaceAlt; border.width: 1
                        MouseArea {
                            anchors.fill: parent
                            onClicked: doToggleTorrentFile(selectedTorrents[0].id, modelData)
                        }
                    }
                    Text { text: modelData.path || "-"; color: Theme.text; font.pixelSize: Theme.fontSizeSmall; elide: Text.ElideRight; width: 380 }
                    Text { text: ((modelData.size || 0) / (1024*1024)).toFixed(1) + " MB"; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                }
            }
        }

        // Trackers tab (FQT-034)
        Item {
            width: parent.width
            height: parent.height - 53
            visible: torrentDetailTab.activeTab === 2

            ListView {
                anchors { fill: parent; margins: Theme.sp2 }
                model: torrentDetailGeneral.tdData ? torrentDetailGeneral.tdData.trackers : []
                clip: true
                cacheBuffer: 200

                Text {
                    visible: (torrentDetailGeneral.tdData ? torrentDetailGeneral.tdData.trackers.length : 0) === 0
                    anchors.centerIn: parent
                    text: qsTr("No trackers"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
                }

                delegate: Row {
                    spacing: Theme.sp2; height: 20; width: ListView.view.width
                    Text { text: modelData.url || "-"; color: Theme.text; font.pixelSize: Theme.fontSizeSmall; elide: Text.ElideRight; width: 340 }
                    Text { text: modelData.status || "-"; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 70 }
                    Text { text: modelData.last_error || ""; color: Theme.danger; font.pixelSize: Theme.fontSizeSmall; elide: Text.ElideRight; width: 150 }
                }
            }
        }

        // Peers tab (FQT-034) \u2014 read-only, geo + eyes_tier color coding
        Item {
            id: torrentDetailPeers
            width: parent.width
            height: parent.height - 53
            visible: torrentDetailTab.activeTab === 3

            // LivePeerView.eyes_tier: five|nine|fourteen|other|local|unknown
            function peerEyesColor(tier, fallback) {
                switch (tier || "") {
                case "five": return Theme.danger
                case "nine": return Theme.warning
                case "fourteen": return Theme.warning
                case "local": return Theme.success
                case "unknown": return Theme.textDim
                default: return fallback
                }
            }

            function countEyesTier(tier) {
                var n = 0
                for (var i = 0; i < torrentPeers.length; i++) {
                    if ((torrentPeers[i].eyes_tier || "") === tier) n++
                }
                return n
            }

            readonly property int fiveEyesCount: {
                var _ = torrentPeers
                return countEyesTier("five")
            }
            readonly property int nineEyesCount: {
                var _ = torrentPeers
                return countEyesTier("nine")
            }
            readonly property int fourteenEyesCount: {
                var _ = torrentPeers
                return countEyesTier("fourteen")
            }
            readonly property bool hasEyesPeers: fiveEyesCount > 0 || nineEyesCount > 0 || fourteenEyesCount > 0

            Column {
                anchors { fill: parent; margins: Theme.sp2 }
                spacing: Theme.sp1

                Text {
                    width: parent.width
                    visible: torrentDetailPeers.hasEyesPeers
                    height: visible ? implicitHeight : 0
                    text: torrentDetailPeers.fiveEyesCount + " five-eyes / "
                          + torrentDetailPeers.nineEyesCount + " nine-eyes peers"
                    color: Theme.warning
                    font.pixelSize: Theme.fontSizeSmall
                }

                ListView {
                    width: parent.width
                    height: parent.height - (torrentDetailPeers.hasEyesPeers ? 18 : 0)
                    model: torrentPeers
                    clip: true
                    cacheBuffer: 200

                    Text {
                        visible: torrentPeers.length === 0
                        anchors.centerIn: parent
                        text: torrentPeersError !== "" ? torrentPeersError : "No peers"
                        color: torrentPeersError !== "" ? Theme.danger : Theme.textDim
                        font.pixelSize: Theme.fontSizeSmall
                    }

                    delegate: Row {
                        spacing: Theme.sp2; height: 20; width: ListView.view.width
                        readonly property string eyesTier: modelData.eyes_tier || ""
                        Text {
                            text: (modelData.flag_emoji || "") + " " + (modelData.client || "-")
                            color: torrentDetailPeers.peerEyesColor(parent.eyesTier, Theme.text)
                            font.pixelSize: Theme.fontSizeSmall; elide: Text.ElideRight; width: 220
                        }
                        Text {
                            text: modelData.country_name || "-"
                            color: torrentDetailPeers.peerEyesColor(parent.eyesTier, Theme.textDim)
                            font.pixelSize: Theme.fontSizeSmall; width: 110
                        }
                        Text {
                            text: "↓" + ((modelData.downloaded || 0) / (1024*1024)).toFixed(1)
                                  + " ↑" + ((modelData.uploaded || 0) / (1024*1024)).toFixed(1) + " MB"
                            color: torrentDetailPeers.peerEyesColor(parent.eyesTier, Theme.textDim)
                            font.pixelSize: Theme.fontSizeSmall
                        }
                    }
                }
            }
        }

        // FQT-075: Pieces tab
        Item {
            width: parent.width
            height: parent.height - 53
            visible: torrentDetailTab.activeTab === 4

            Rectangle {
                id: piecesCanvas
                anchors { fill: parent; margins: Theme.sp2 }
                color: Theme.bg; radius: 3

                // FQT-075 honesty: use real piece rows from detail when present.
                readonly property var pieceList: (torrentDetailGeneral.tdData && torrentDetailGeneral.tdData.pieces)
                    ? torrentDetailGeneral.tdData.pieces : []
                readonly property int pieceCount: pieceList.length

                Text {
                    anchors.centerIn: parent
                    width: parent.width - 24
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.WordWrap
                    visible: piecesCanvas.pieceCount === 0
                    text: qsTr("Pieces map not available for this torrent (engine does not expose piece status). Not a loading error.")
                    color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
                }

                Grid {
                    anchors.centerIn: parent
                    visible: piecesCanvas.pieceCount > 0
                    columns: 32
                    spacing: 1
                    Repeater {
                        model: Math.min(piecesCanvas.pieceCount, 512)
                        Rectangle {
                            width: 4; height: 4; radius: 1
                            color: {
                                var p = piecesCanvas.pieceList[index]
                                if (!p) return Theme.surfaceAlt
                                var st = (p.status || "").toLowerCase()
                                var have = st === "have" || st === "complete" || st === "done"
                                return have ? Theme.success : Theme.surfaceAlt
                            }
                        }
                    }
                }

                Text {
                    anchors.bottom: parent.bottom; anchors.right: parent.right; anchors.margins: Theme.sp1
                    text: piecesCanvas.pieceCount > 0
                          ? (piecesCanvas.pieceCount + qsTr(" pieces"))
                          : qsTr("Pieces not available")
                    color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
                }
            }
        }

        // HTTP Sources tab
        Item {
            width: parent.width
            height: parent.height - 53
            visible: torrentDetailTab.activeTab === 5

            Column {
                anchors { fill: parent; margins: Theme.sp2 }
                spacing: Theme.sp1
                Text {
                    text: qsTr("HTTP Sources (Web Seeds)")
                    color: Theme.text; font.pixelSize: Theme.fontSizeNormal; font.bold: true
                }
                Text {
                    text: qsTr("No web seeds available for this torrent.")
                    color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; wrapMode: Text.Wrap; width: parent.width
                }
                Text {
                    text: qsTr("Web seeds (BEP-19) are listed here when the .torrent file includes url-list entries. Most torrents do not use web seeds.")
                    color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; wrapMode: Text.Wrap; width: parent.width
                }
            }
        }

        // Speed tab
        Item {
            width: parent.width
            height: parent.height - 53
            visible: torrentDetailTab.activeTab === 6

            Column {
                anchors { fill: parent; margins: Theme.sp2 }
                spacing: Theme.sp2

                Row {
                    spacing: Theme.sp4
                    Column {
                        spacing: 2
                        Text { text: qsTr("Download"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                        Text {
                            color: Theme.success; font.pixelSize: Theme.fontSizeNormal; font.bold: true
                            text: {
                                if (!torrentDetailGeneral.tdData) return "0 B/s"
                                var sp = torrentDetailGeneral.tdData.download_speed || 0
                                return sp >= 1048576 ? (sp / 1048576).toFixed(1) + " MB/s"
                                     : sp >= 1024 ? (sp / 1024).toFixed(0) + " KB/s"
                                     : sp + " B/s"
                            }
                        }
                    }
                    Column {
                        spacing: 2
                        Text { text: qsTr("Upload"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                        Text {
                            color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true
                            text: {
                                if (!torrentDetailGeneral.tdData) return "0 B/s"
                                var sp = torrentDetailGeneral.tdData.upload_speed || 0
                                return sp >= 1048576 ? (sp / 1048576).toFixed(1) + " MB/s"
                                     : sp >= 1024 ? (sp / 1024).toFixed(0) + " KB/s"
                                     : sp + " B/s"
                            }
                        }
                    }
                }

                Grid {
                    columns: 2; columnSpacing: Theme.sp3; rowSpacing: Theme.sp1
                    Text { text: qsTr("Total Downloaded:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    Text {
                        color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                        text: {
                            if (!torrentDetailGeneral.tdData) return "-"
                            var d = torrentDetailGeneral.tdData.downloaded || 0
                            return d >= 1073741824 ? (d / 1073741824).toFixed(2) + " GB"
                                 : d >= 1048576 ? (d / 1048576).toFixed(1) + " MB"
                                 : d >= 1024 ? (d / 1024).toFixed(0) + " KB"
                                 : d + " B"
                        }
                    }
                    Text { text: qsTr("Total Uploaded:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    Text {
                        color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                        text: {
                            if (!torrentDetailGeneral.tdData) return "-"
                            var u = torrentDetailGeneral.tdData.uploaded || 0
                            return u >= 1073741824 ? (u / 1073741824).toFixed(2) + " GB"
                                 : u >= 1048576 ? (u / 1048576).toFixed(1) + " MB"
                                 : u >= 1024 ? (u / 1024).toFixed(0) + " KB"
                                 : u + " B"
                        }
                    }
                    Text { text: qsTr("Share Ratio:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    Text {
                        color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                        text: torrentDetailGeneral.tdData ? (torrentDetailGeneral.tdData.ratio || "0.00") : "-"
                    }
                }

                // Speed history placeholder
                Rectangle {
                    width: parent.width; height: 80; radius: 4
                    color: Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                    Text {
                        anchors.centerIn: parent
                        text: qsTr("Speed history graph \u2014 coming soon")
                        color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
                    }
                }
            }
        }
    }
}
