import QtQuick
import QtQuick.Window
import com.ferric.app 1.0

// IDM-style "Download status" window: status block, overall progress bar and the
// "Start positions and download progress by connections" segment map + list.
// Polls its own download once a second; closing never cancels the download.
Window {
    id: win

    property string downloadId: ""
    property var requestBus
    property var dl: null
    property var chunks: []
    property bool detailsShown: true
    property real rate: 0          // bytes/s, smoothed
    property real lastBytes: -1
    property real lastStamp: 0
    property bool announced: false
    property int tick: 0

    signal finished(string id, var info)

    readonly property string status: dl ? dl.status : ""
    readonly property real total: dl && dl.total_size ? dl.total_size : 0
    readonly property real done: dl ? dl.downloaded : 0
    readonly property real fraction: total > 0 ? Math.min(1, done / total) : 0

    width: 560
    height: detailsShown ? 534 : 292
    minimumWidth: 480
    minimumHeight: detailsShown ? 474 : 292
    color: Theme.bg
    title: (total > 0 ? Math.floor(fraction * 100) + "% " : "") + (dl ? dl.filename : qsTr("Download status"))

    function fmtBytes(n) {
        if (n === undefined || n === null || n <= 0) return "0 KB";
        if (n < 1024 * 1024) return (n / 1024).toFixed(2) + " KB";
        if (n < 1024 * 1024 * 1024) return (n / 1048576).toFixed(2) + " MB";
        return (n / 1073741824).toFixed(2) + " GB";
    }

    function fmtTime(sec) {
        if (!isFinite(sec) || sec < 0) return "";
        sec = Math.round(sec);
        var h = Math.floor(sec / 3600), m = Math.floor(sec % 3600 / 60), s = sec % 60;
        if (h > 0) return h + " hour " + m + " min " + s + " sec";
        if (m > 0) return m + " min " + s + " sec";
        return s + " sec";
    }

    function statusText() {
        switch (status) {
        case "connecting": return qsTr("Connecting...");
        case "downloading": return qsTr("Receiving data...");
        case "paused": return qsTr("Paused");
        case "queued": return qsTr("Queued");
        case "merging": return qsTr("Merging...");
        case "completed": return qsTr("Download complete");
        case "error": return (dl && dl.error_msg) ? dl.error_msg : qsTr("Error");
        case "cancelled": return qsTr("Cancelled");
        }
        return "";
    }

    function refresh() {
        if (!downloadId || !requestBus) return;
        var id = downloadId;
        requestBus("get_download", { id: id }, function(raw) {
            if (id !== win.downloadId) return;
            var d;
            try { d = JSON.parse(raw); } catch (e) { return; }
            if (!d || d.error) return;
            var now = Date.now();
            if (win.lastBytes >= 0 && now > win.lastStamp && d.status === "downloading") {
                var dt = now - win.lastStamp;
                var inst = (d.downloaded - win.lastBytes) * 1000 / dt;
                // exponential average with a ~2 s time constant, independent of the poll interval
                win.rate = win.rate > 0 ? win.rate + (1 - Math.exp(-dt / 2000)) * (inst - win.rate) : inst;
            } else if (d.status !== "downloading") {
                win.rate = 0;
            }
            win.lastBytes = d.downloaded;
            win.lastStamp = now;
            win.dl = d;
            if (d.status === "completed" && !win.announced) {
                win.announced = true;
                win.finished(id, d);
            }
        });
        if (detailsShown && (tick++ % 4 === 0)) {
            requestBus("get_download_chunks", { id: id }, function(raw) {
                if (id !== win.downloadId) return;
                try { win.chunks = JSON.parse(raw); } catch (e) { win.chunks = []; }
            });
        }
    }

    Timer {
        interval: 250
        repeat: true
        running: win.visible && win.status !== "completed"
        triggeredOnStart: true
        onTriggered: win.refresh()
    }

    onDownloadIdChanged: { announced = false; lastBytes = -1; rate = 0; dl = null; chunks = []; refresh(); }

    Column {
        anchors { fill: parent; margins: Theme.sp3 }
        spacing: Theme.sp2

        Text {
            width: parent.width
            text: dl ? (dl.final_url || dl.url) : ""
            textFormat: Text.PlainText
            elide: Text.ElideMiddle
            color: Theme.textDim
            font.pixelSize: Theme.fontSizeSmall
        }

        Rectangle { width: parent.width; height: 1; color: Theme.surfaceAlt }

        // status block: label / value pairs as in IDM's download status tab
        Grid {
            columns: 2
            columnSpacing: Theme.sp3
            rowSpacing: Theme.sp1
            Text { text: qsTr("Status"); color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal; width: 130 }
            Text { text: win.statusText(); color: win.status === "error" ? Theme.danger : Theme.text; font.pixelSize: Theme.fontSizeNormal; width: win.width - 190; elide: Text.ElideRight }
            Text { text: qsTr("File size"); color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal }
            Text { text: win.total > 0 ? win.fmtBytes(win.total) : qsTr("unknown"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
            Text { text: qsTr("Downloaded"); color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal }
            Text {
                text: win.fmtBytes(win.done) + (win.total > 0 ? "  (" + (win.fraction * 100).toFixed(2) + " %)" : "")
                color: Theme.text; font.pixelSize: Theme.fontSizeNormal
            }
            Text { text: qsTr("Transfer rate"); color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal }
            Text { // a blank value would be skipped by the Grid (zero-size item) and shift every later row
                text: win.rate <= 0 ? " " : (win.rate >= 1048576 ? (win.rate / 1048576).toFixed(2) + " MB/sec" : (win.rate / 1024).toFixed(2) + " KB/sec"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
            Text { text: qsTr("Time left"); color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal }
            Text { text: win.rate > 0 && win.total > 0 ? win.fmtTime((win.total - win.done) / win.rate) : " "; color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
            Text { text: qsTr("Resume capability"); color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal }
            Text {
                text: win.chunks.length > 1 ? qsTr("Yes") : (win.dl && win.status !== "connecting" && win.total > 0 && win.chunks.length === 1 ? qsTr("No") : qsTr("(unknown)"))
                color: Theme.text; font.pixelSize: Theme.fontSizeNormal
            }
        }

        // overall progress bar
        Rectangle {
            width: parent.width; height: Theme.rowH; radius: Theme.radius
            color: Theme.surface; border.color: Theme.border
            Rectangle {
                width: parent.width * win.fraction; height: parent.height; radius: Theme.radius
                color: win.status === "error" ? Theme.danger : Theme.accent
                Behavior on width { NumberAnimation { duration: 260; easing.type: Easing.Linear } }
            }
            Text {
                anchors.centerIn: parent
                text: Math.floor(win.fraction * 100) + " %"
                color: win.fraction > 0.5 ? "white" : Theme.text
                font.pixelSize: Theme.fontSizeSmall
            }
        }

        // IDM keeps "Hide details" and the two action buttons on one row
        Item {
            width: parent.width; height: Theme.controlH
            Rectangle {
                width: 110; height: Theme.controlH; radius: Theme.radius; color: Theme.surfaceAlt
                Text { anchors.centerIn: parent; text: win.detailsShown ? qsTr("<< Hide details") : qsTr("Show details >>"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { anchors.fill: parent; onClicked: { win.detailsShown = !win.detailsShown; if (win.detailsShown) win.refresh(); } }
            }
            Row {
                anchors.right: parent.right
                spacing: Theme.sp2
                Rectangle {
                    width: 80; height: Theme.controlH; radius: Theme.radius; color: Theme.surfaceAlt
                    visible: win.status !== "completed"
                    Text { anchors.centerIn: parent; text: win.status === "paused" || win.status === "error" ? qsTr("Resume") : qsTr("Pause"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea {
                        anchors.fill: parent
                        onClicked: win.requestBus("download_action", {
                            id: win.downloadId,
                            action: (win.status === "paused" || win.status === "error") ? "resume" : "pause"
                        }, function() { win.refresh(); })
                    }
                }
                Rectangle {
                    width: 80; height: Theme.controlH; radius: Theme.radius; color: Theme.accent
                    Text { anchors.centerIn: parent; text: win.status === "completed" ? qsTr("Close") : qsTr("Cancel"); color: "white"; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea {
                        anchors.fill: parent
                        onClicked: {
                            // IDM's Cancel stops the transfer but keeps the entry and partial file.
                            if (win.status !== "completed")
                                win.requestBus("download_action", { id: win.downloadId, action: "pause" }, function() {});
                            win.close();
                        }
                    }
                }
            }
        }

        // Per-download speed limiter, reduced to one row: KBytes/sec for this download, 0 = unlimited
        Row {
            visible: win.detailsShown && win.status !== "completed"
            spacing: Theme.sp2
            Text { text: qsTr("Maximum download speed (KB/sec, 0 = no limit):"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; anchors.verticalCenter: parent.verticalCenter }
            Rectangle {
                anchors.verticalCenter: parent.verticalCenter
                width: 70; height: Theme.controlH; color: Theme.surface; border.color: limitInput.activeFocus ? Theme.accent : Theme.border
                TextInput {
                    id: limitInput
                    anchors { fill: parent; margins: Theme.sp1 }
                    color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                    text: win.dl && win.dl.speed_limit_bps > 0 ? Math.round(win.dl.speed_limit_bps / 1024).toString() : "0"
                    inputMethodHints: Qt.ImhDigitsOnly
                    onEditingFinished: win.requestBus("set_download_speed_limit", {
                        id: win.downloadId,
                        bps: (parseInt(text) || 0) * 1024
                    }, function() {})
                }
            }
        }

        Text {
            visible: win.detailsShown
            text: qsTr("Start positions and download progress by connections")
            color: Theme.text; font.pixelSize: Theme.fontSizeNormal; font.bold: true
        }

        // segment map: pink = start position of a segment, blue = downloaded, white = still to fetch
        Rectangle {
            id: segMap
            visible: win.detailsShown
            width: parent.width; height: Theme.rowH
            color: "white"; border.color: Theme.border
            clip: true
            Repeater {
                model: win.total > 0 ? win.chunks.length : 0
                delegate: Item {
                    readonly property var c: win.chunks[index]
                    readonly property real len: c ? c.byte_end - c.byte_start + 1 : 1
                    x: c ? segMap.width * c.byte_start / win.total : 0
                    width: segMap.width * len / win.total
                    height: segMap.height
                    Rectangle {
                        width: c ? parent.width * Math.min(1, c.downloaded / len) : 0
                        height: parent.height
                        color: "#2f6fd6"
                        Behavior on width { NumberAnimation { duration: 260; easing.type: Easing.Linear } }
                    }
                    Rectangle { width: 1; height: parent.height; color: "#e91e8c" }
                }
            }
        }

        // connections list: N. / Downloaded / Info
        Rectangle {
            visible: win.detailsShown
            width: parent.width
            height: win.detailsShown ? parent.height - y - 0 : 0
            color: Theme.surface; border.color: Theme.border
            clip: true
            Row {
                id: connHeader
                height: Theme.rowH
                Text { text: qsTr("N."); width: 40; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; leftPadding: Theme.sp1; verticalAlignment: Text.AlignVCenter; height: parent.height }
                Text { text: qsTr("Downloaded"); width: 130; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; verticalAlignment: Text.AlignVCenter; height: parent.height }
                Text { text: qsTr("Info"); width: 200; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; verticalAlignment: Text.AlignVCenter; height: parent.height }
            }
            ListView {
                anchors { top: connHeader.bottom; left: parent.left; right: parent.right; bottom: parent.bottom }
                clip: true
                model: win.chunks.length
                delegate: Row {
                    readonly property var c: win.chunks[index]
                    height: Theme.rowH
                    Text { text: c ? (c.chunk_index + 1) + "." : ""; width: 40; color: Theme.text; font.pixelSize: Theme.fontSizeSmall; leftPadding: Theme.sp1 }
                    Text { text: c ? (c.downloaded / 1024).toFixed(3) + " KB" : ""; width: 130; color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                    Text {
                        width: 200; color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                        text: !c ? "" : c.status === "completed" ? qsTr("Download complete")
                            : c.status === "downloading" ? qsTr("Receiving data...")
                            : c.status === "error" ? qsTr("Error") : qsTr("Connecting...")
                    }
                }
            }
        }
    }
}
