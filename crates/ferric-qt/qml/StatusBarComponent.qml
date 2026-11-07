import QtQuick

// StatusBar component \u2014 extracted from main.qml (Phase 2 split #22)
Rectangle {
    id: statusBar

    height: Theme.rowH
    color: Theme.bg
    border.color: Theme.surfaceAlt
    border.width: 1

    property bool connected: false
    property int port: 0
    property bool hubLive: false
    property real downloadBps: 0
    property real uploadBps: 0
    property int activeDownloads: 0
    property int totalDownloads: 0
    property int activeTorrents: 0
    property int totalTorrents: 0
    property string mode: "balanced"
    property bool killSwitchActive: false
    property bool torrentsVisible: false

    function formatBps(bps) {
        if (!bps || bps === 0) return "0 B/s";
        if (bps < 1024) return bps.toFixed(0) + " B/s";
        if (bps < 1048576) return (bps / 1024).toFixed(1) + " KB/s";
        if (bps < 1073741824) return (bps / 1048576).toFixed(1) + " MB/s";
        return (bps / 1073741824).toFixed(2) + " GB/s";
    }

    Row {
        anchors.verticalCenter: parent.verticalCenter
        anchors.left: parent.left
        anchors.leftMargin: Theme.sp2
        spacing: Theme.sp3

        // Connection status
        Row {
            spacing: Theme.sp1
            Rectangle {
                width: 8; height: 8; radius: 4
                color: statusBar.connected ? Theme.success : Theme.danger
                anchors.verticalCenter: parent.verticalCenter
            }
            Text {
                text: statusBar.connected ? "Connected" : "Disconnected"
                color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
                anchors.verticalCenter: parent.verticalCenter
            }
        }

        // Speed
        Text {
            text: "↓ " + statusBar.formatBps(statusBar.downloadBps) + (statusBar.torrentsVisible ? "  ↑ " + statusBar.formatBps(statusBar.uploadBps) : "")
            color: Theme.text; font.pixelSize: Theme.fontSizeSmall
            anchors.verticalCenter: parent.verticalCenter
        }

        // Counts
        Text {
            text: statusBar.activeDownloads + "/" + statusBar.totalDownloads + " downloads" +
                  (statusBar.torrentsVisible ? "  " + statusBar.activeTorrents + "/" + statusBar.totalTorrents + " BT" : "")
            color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
            anchors.verticalCenter: parent.verticalCenter
        }

        // Mode
        Text {
            visible: statusBar.torrentsVisible
            text: "Mode: " + (statusBar.mode === "strict" ? "Strict" : statusBar.mode === "open" ? "Open" : "Balanced")
            color: statusBar.mode === "strict" ? Theme.warning : statusBar.mode === "open" ? Theme.accent : Theme.success
            font.pixelSize: Theme.fontSizeSmall; font.bold: true
            anchors.verticalCenter: parent.verticalCenter
        }

        // Kill switch
        Text {
            visible: statusBar.torrentsVisible && statusBar.killSwitchActive
            text: "KILL SWITCH"
            color: Theme.danger; font.pixelSize: Theme.fontSizeSmall; font.bold: true
            anchors.verticalCenter: parent.verticalCenter
        }
    }

    // SSE status (right side)
    Text {
        anchors.verticalCenter: parent.verticalCenter
        anchors.right: parent.right
        anchors.rightMargin: Theme.sp2
        text: statusBar.hubLive ? "Live" : "Refreshing"
        color: statusBar.hubLive ? Theme.success : Theme.textDim; font.pixelSize: Theme.fontSizeSmall
    }
}
