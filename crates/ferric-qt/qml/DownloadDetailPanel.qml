import QtQuick
import QtQuick.Controls
import com.ferric.app 1.0

// FQT-025: download detail panel - extracted from main.qml (Phase 2 split)
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
    property var selectedDownloads: []

    // Parent-owned callbacks (dialogs stay in main.qml)
    property var doOpenFolder
    property var doOpenFile
    property var doCopyUrl
    property var doCopyName
    property var doShowRefreshUrl
    property var doPriorityUp
    property var doPriorityDown
    property var doShowSpeedLimit

    function refreshDetail() {
        if (selectedDownloads.length !== 1) return
        var id = selectedDownloads[0].id
        requestBus("get_download", { id: id }, function(raw) {
            if (selectedDownloads.length !== 1 || selectedDownloads[0].id !== id) return
            try { detailGeneral.dlData = JSON.parse(raw) } catch(e) { detailGeneral.dlData = null }
        })
        if (detailTab.activeTab === 1) {
            requestBus("get_download_chunks", { id: id }, function(raw) {
                if (selectedDownloads.length !== 1 || selectedDownloads[0].id !== id) return
                try { detailChunks.chunksData = JSON.parse(raw) } catch(e) { detailChunks.chunksData = [] }
            })
        }
        if (detailTab.activeTab === 2) {
            requestBus("get_download_audit", { id: id }, function(raw) {
                if (selectedDownloads.length !== 1 || selectedDownloads[0].id !== id) return
                try { detailAudit.auditData = JSON.parse(raw) } catch(e) { detailAudit.auditData = [] }
            })
        }
        if (detailTab.activeTab === 3) {
            requestBus("get_download_diagnostic", { id: id }, function(raw) {
                if (selectedDownloads.length !== 1 || selectedDownloads[0].id !== id) return
                try { detailDiagnostic.diagData = JSON.parse(raw) } catch(e) { detailDiagnostic.diagData = null }
            })
        }
    }

    onVisibleChanged: if (visible) refreshDetail()
    onSelectedDownloadsChanged: if (visible) refreshDetail()

    Connections {
        target: hub
        function onUpdated() { if (panel.visible) refreshDetail() }
    }

    // SSE only fires on state changes, so size/chunks went stale while a download ran
    // (9.0 / 20.0 MB shown at 84%). Poll while the selected download is active.
    Timer {
        interval: 1500
        repeat: true
        running: panel.visible && detailGeneral.dlData
            && (detailGeneral.dlData.status === "downloading" || detailGeneral.dlData.status === "connecting")
        onTriggered: panel.refreshDetail()
    }
    Column {
        anchors.fill: parent
        spacing: 0

        // tab bar
        // ponytail: Item (not Row) - a Row positioner ignores anchors on
        // its children, so the right-anchored action-button Row below
        // never actually right-aligned (real bug, caught live: QML
        // warned "Row will not function" and the buttons just fell into
        // the normal left-to-right flow next to the tabs).
        Item {
            id: detailTab
            property int activeTab: 0
            width: parent.width
            height: 52

            Row {
                anchors.left: parent.left
                Rectangle {
                    width: 90; height: 28
                    color: detailTab.activeTab === 0 ? Theme.surface : "transparent"
                    border.color: detailTab.activeTab === 0 ? Theme.accent : "transparent"
                    border.width: detailTab.activeTab === 0 ? 0 : 0
                    MouseArea { anchors.fill: parent; onClicked: { detailTab.activeTab = 0 } }
                    Text { anchors.centerIn: parent; text: qsTr("General"); color: detailTab.activeTab === 0 ? Theme.text : Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                }
                Rectangle {
                    width: 90; height: 28
                    color: detailTab.activeTab === 1 ? Theme.surface : "transparent"
                    MouseArea { anchors.fill: parent; onClicked: { detailTab.activeTab = 1; refreshDetail() } }
                    Text { anchors.centerIn: parent; text: qsTr("Chunks"); color: detailTab.activeTab === 1 ? Theme.text : Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                }
                Rectangle {
                    width: 90; height: 28
                    color: detailTab.activeTab === 2 ? Theme.surface : "transparent"
                    MouseArea { anchors.fill: parent; onClicked: { detailTab.activeTab = 2; refreshDetail() } }
                    Text { anchors.centerIn: parent; text: qsTr("Audit"); color: detailTab.activeTab === 2 ? Theme.text : Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                }
                Rectangle {
                    width: 90; height: 28
                    color: detailTab.activeTab === 3 ? Theme.surface : "transparent"
                    MouseArea { anchors.fill: parent; onClicked: { detailTab.activeTab = 3; refreshDetail() } }
                    Text { anchors.centerIn: parent; text: qsTr("Diagnostic"); color: detailTab.activeTab === 3 ? Theme.text : Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                }
            }
            // FQT-027/029 action buttons
            Row {
                anchors.left: parent.left
                anchors.leftMargin: Theme.sp2
                anchors.top: parent.top
                anchors.topMargin: Theme.sp5
                spacing: Theme.sp1
                Rectangle {
                    width: 76; height: 20; radius: 3; color: openFolderArea.containsMouse ? Theme.surfaceAlt : Theme.surface
                    visible: selectedDownloads.length === 1
                    Text { anchors.centerIn: parent; text: qsTr("Open Folder"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea { id: openFolderArea; anchors.fill: parent; hoverEnabled: true; onClicked: doOpenFolder() }
                }
                Rectangle {
                    width: 60; height: 20; radius: 3; color: openFileArea.containsMouse ? Theme.surfaceAlt : Theme.surface
                    visible: selectedDownloads.length === 1
                    Text { anchors.centerIn: parent; text: qsTr("Open File"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea { id: openFileArea; anchors.fill: parent; hoverEnabled: true; onClicked: doOpenFile() }
                }
                Rectangle {
                    width: 60; height: 20; radius: 3; color: copyUrlArea.containsMouse ? Theme.surfaceAlt : Theme.surface
                    visible: selectedDownloads.length === 1
                    Text { anchors.centerIn: parent; text: qsTr("Copy URL"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea { id: copyUrlArea; anchors.fill: parent; hoverEnabled: true; onClicked: doCopyUrl() }
                }
                Rectangle {
                    width: 68; height: 20; radius: 3; color: copyNameArea.containsMouse ? Theme.surfaceAlt : Theme.surface
                    visible: selectedDownloads.length === 1
                    Text { anchors.centerIn: parent; text: qsTr("Copy Name"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea { id: copyNameArea; anchors.fill: parent; hoverEnabled: true; onClicked: doCopyName() }
                }
                Rectangle {
                    width: 76; height: 20; radius: 3; color: refreshUrlBtnArea.containsMouse ? Theme.surfaceAlt : Theme.surface
                    visible: selectedDownloads.length === 1
                    Text { anchors.centerIn: parent; text: qsTr("Refresh URL"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea { id: refreshUrlBtnArea; anchors.fill: parent; hoverEnabled: true; onClicked: doShowRefreshUrl() }
                }
                Rectangle {
                    width: 20; height: 20; radius: 3; color: prioUpArea.containsMouse ? Theme.surfaceAlt : Theme.surface
                    visible: selectedDownloads.length === 1
                    Text { anchors.centerIn: parent; text: qsTr("\u25b2"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea { id: prioUpArea; anchors.fill: parent; hoverEnabled: true; onClicked: doPriorityUp() }
                }
                Rectangle {
                    width: 20; height: 20; radius: 3; color: prioDownArea.containsMouse ? Theme.surfaceAlt : Theme.surface
                    visible: selectedDownloads.length === 1
                    Text { anchors.centerIn: parent; text: qsTr("\u25bc"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea { id: prioDownArea; anchors.fill: parent; hoverEnabled: true; onClicked: doPriorityDown() }
                }
                Rectangle {
                    width: 58; height: 20; radius: 3; color: speedLimArea.containsMouse ? Theme.surfaceAlt : Theme.surface
                    Text { anchors.centerIn: parent; text: qsTr("Set Limit"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea { id: speedLimArea; anchors.fill: parent; hoverEnabled: true; onClicked: doShowSpeedLimit() }
                }
            }
        }

        Rectangle { width: parent.width; height: 1; color: Theme.surfaceAlt }

        // General tab
        Item {
            id: detailGeneral
            property var dlData: null
            width: parent.width
            height: parent.height - 53
            visible: detailTab.activeTab === 0

            Grid {
                anchors { fill: parent; margins: Theme.sp2 }
                columns: 2
                columnSpacing: Theme.sp3
                rowSpacing: Theme.sp1

                Text { text: qsTr("File:");   color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Text { text: detailGeneral.dlData ? (detailGeneral.dlData.filename || "-") : "-"; textFormat: Text.PlainText; color: Theme.text; font.pixelSize: Theme.fontSizeSmall; elide: Text.ElideRight; width: 320 }
                Text { text: qsTr("URL:");    color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Text { text: detailGeneral.dlData ? (detailGeneral.dlData.url || "-") : "-"; textFormat: Text.PlainText; color: Theme.text; font.pixelSize: Theme.fontSizeSmall; elide: Text.ElideRight; width: 320 }
                Text { text: qsTr("Folder:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Text {
                    // The record carries the full file path; show its folder.
                    text: {
                        if (!detailGeneral.dlData) return "-";
                        var p = detailGeneral.dlData.save_path || "";
                        var cut = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
                        return cut > 0 ? p.substring(0, cut) : (detailGeneral.dlData.save_dir || "-");
                    }
                    color: Theme.text; font.pixelSize: Theme.fontSizeSmall; elide: Text.ElideRight; width: 320
                }
                Text { text: qsTr("Status:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Text { text: detailGeneral.dlData ? (detailGeneral.dlData.status || "-") : "-"; color: Theme.accent; font.pixelSize: Theme.fontSizeSmall }
                Text { text: qsTr("Size:");   color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Text {
                    color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                    text: {
                        if (!detailGeneral.dlData) return "-"
                        var t = detailGeneral.dlData.total_size || 0
                        var d = detailGeneral.dlData.downloaded || 0
                        return (t > 0 ? (d / (1024*1024)).toFixed(1) + " / " + (t / (1024*1024)).toFixed(1) + " MB" : "-")
                    }
                }
                Text { text: qsTr("Chunks:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Text { text: detailGeneral.dlData ? ((detailGeneral.dlData.chunk_count || 0) + " chunks") : "-"; color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                Text { text: qsTr("Description:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                TextField {
                    clip: true
                    id: descField
                    width: 320; height: 20
                    color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                    background: Rectangle { color: Theme.surface; border.color: Theme.surfaceAlt; radius: 2 }
                    text: detailGeneral.dlData ? (detailGeneral.dlData.description || "") : ""
                    onEditingFinished: {
                        if (selectedDownloads.length === 1)
                            bus.set_download_description(selectedDownloads[0].id, text)
                    }
                }
            }
        }

        // Chunks tab
        Item {
            id: detailChunks
            property var chunksData: []
            width: parent.width
            height: parent.height - 53
            visible: detailTab.activeTab === 1

            ListView {
                anchors { fill: parent; margins: Theme.sp2 }
                model: detailChunks.chunksData.length ? detailChunks.chunksData : []
                clip: true
                cacheBuffer: 200
                delegate: Row {
                    spacing: Theme.sp2; height: 18; width: ListView.view.width
                    Text { text: "#" + (modelData.chunk_index || 0); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 30 }
                    Text { text: modelData.status || "-"; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 80 }
                    Text {
                        color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                        text: {
                            var d = modelData.downloaded || 0; var t = (modelData.byte_end || 0) - (modelData.byte_start || 0)
                            return t > 0 ? ((d/(1024)).toFixed(0) + "/" + (t/1024).toFixed(0) + " KB") : "-"
                        }
                    }
                }
            }
        }

        // FQT-051: Audit tab
        Item {
            id: detailAudit
            property var auditData: []
            width: parent.width
            height: parent.height - 53
            visible: detailTab.activeTab === 2

            ListView {
                anchors { fill: parent; margins: Theme.sp2 }
                model: detailAudit.auditData.length ? detailAudit.auditData : []
                clip: true
                cacheBuffer: 200
                delegate: Row {
                    spacing: Theme.sp2; height: 18; width: ListView.view.width
                    Text { text: modelData.event_type || "-"; color: Theme.accent; font.pixelSize: Theme.fontSizeSmall; width: 100 }
                    Text { text: modelData.detail_json || "-"; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; elide: Text.ElideRight; width: 300 }
                    Text {
                        color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
                        text: {
                            var ts = modelData.created_at || 0;
                            var d = new Date(ts);
                            return d.toLocaleTimeString();
                        }
                    }
                }
                Text {
                    visible: detailAudit.auditData.length === 0
                    anchors.centerIn: parent
                    text: qsTr("No audit events")
                    color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
                }
            }
        }

        // FQT-051: Diagnostic tab
        Item {
            id: detailDiagnostic
            property var diagData: null
            width: parent.width
            height: parent.height - 53
            visible: detailTab.activeTab === 3

            Column {
                anchors { fill: parent; margins: Theme.sp2 }
                spacing: Theme.sp1
                visible: detailDiagnostic.diagData !== null

                Row {
                    spacing: Theme.sp2
                    Rectangle {
                        width: 8; height: 8; radius: 4
                        color: detailDiagnostic.diagData && (detailDiagnostic.diagData.severity === "ok" || detailDiagnostic.diagData.severity === "info") ? Theme.success
                            : detailDiagnostic.diagData && detailDiagnostic.diagData.severity === "warning" ? Theme.warning : Theme.danger
                    }
                    Text {
                        color: Theme.text; font.pixelSize: Theme.fontSizeNormal; font.bold: true
                        text: detailDiagnostic.diagData ? detailDiagnostic.diagData.title || "-" : "-"
                    }
                }
                Text {
                    color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; wrapMode: Text.Wrap; width: parent.width
                    text: detailDiagnostic.diagData ? detailDiagnostic.diagData.summary || "-" : "-"
                }
                Text {
                    visible: detailDiagnostic.diagData && detailDiagnostic.diagData.action
                    color: Theme.accent; font.pixelSize: Theme.fontSizeSmall; wrapMode: Text.Wrap; width: parent.width
                    text: detailDiagnostic.diagData && detailDiagnostic.diagData.action ? "Action: " + detailDiagnostic.diagData.action : ""
                }
                Text {
                    color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
                    text: detailDiagnostic.diagData ? "Confidence: " + (detailDiagnostic.diagData.confidence || 0) + "%" : ""
                }
            }

            // Speed info from ProgressHub
            Rectangle {
                anchors.bottom: parent.bottom; anchors.left: parent.left; anchors.right: parent.right
                height: 30; color: Theme.bg
                visible: detailDiagnostic.diagData !== null
                Row {
                    anchors.centerIn: parent; spacing: Theme.sp3
                    Text {
                        color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
                        text: "Speed: " + ((hub.download_bps || 0) >= 1048576 ? ((hub.download_bps || 0) / 1048576).toFixed(1) + " MB/s" : (hub.download_bps || 0) >= 1024 ? ((hub.download_bps || 0) / 1024).toFixed(0) + " KB/s" : (hub.download_bps || 0) + " B/s")
                    }
                    Text { color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; text: "Active: " + (hub.active_downloads || 0) }
                }
            }

            Text {
                visible: detailDiagnostic.diagData === null
                anchors.centerIn: parent
                text: qsTr("No diagnostic data")
                color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
            }
        }
    }
}
