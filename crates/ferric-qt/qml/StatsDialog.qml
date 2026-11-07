import QtQuick

// FQT-103: Stats window \u2014 extracted from main.qml
AppDialog {
    id: statsDialog

    property var session
    property var statsData: ({})
    property var loadStats
    property var formatBps
    property var formatUptime

    title: qsTr("Transfer Statistics")

    contentItem: [
        Row { spacing: 8
            Rectangle { width: 50; height: 22; radius: 3; color: Theme.surfaceAlt
                Text { anchors.centerIn: parent; text: qsTr("Refresh"); color: Theme.text; font.pixelSize: 10 }
                MouseArea { anchors.fill: parent; onClicked: { if (loadStats) loadStats(); } }
            }
        },

        Text { text: qsTr("Session"); color: Theme.accent; font.pixelSize: 12; font.bold: true },

        Grid { columns: 2; spacing: 8; width: parent.width
            Text { text: qsTr("Active downloads:"); color: Theme.textDim; font.pixelSize: 11; width: 140 }
            Text { text: (statsDialog.statsData.active_downloads || 0).toString(); color: Theme.text; font.pixelSize: 11 }
            Text { text: qsTr("Active torrents:"); color: Theme.textDim; font.pixelSize: 11; width: 140 }
            Text { text: (statsDialog.statsData.active_torrents || 0).toString(); color: Theme.text; font.pixelSize: 11 }
            Text { text: qsTr("Total downloads:"); color: Theme.textDim; font.pixelSize: 11; width: 140 }
            Text { text: (statsDialog.statsData.total_downloads || 0).toString(); color: Theme.text; font.pixelSize: 11 }
            Text { text: qsTr("Total torrents:"); color: Theme.textDim; font.pixelSize: 11; width: 140 }
            Text { text: (statsDialog.statsData.total_torrents || 0).toString(); color: Theme.text; font.pixelSize: 11 }
        },

        Text { text: qsTr("Speed"); color: Theme.accent; font.pixelSize: 12; font.bold: true },

        Grid { columns: 2; spacing: 8; width: parent.width
            Text { text: qsTr("Download speed:"); color: Theme.textDim; font.pixelSize: 11; width: 140 }
            Text { text: formatBps ? formatBps(statsDialog.statsData.download_bps || 0) : ""; color: Theme.success; font.pixelSize: 11; font.bold: true }
            Text { text: qsTr("Upload speed:"); color: Theme.textDim; font.pixelSize: 11; width: 140 }
            Text { text: formatBps ? formatBps(statsDialog.statsData.upload_bps || 0) : ""; color: Theme.accent; font.pixelSize: 11; font.bold: true }
        },

        Text { text: qsTr("Connection"); color: Theme.accent; font.pixelSize: 12; font.bold: true },

        Row { spacing: 8; height: 22
            Rectangle { width: 8; height: 8; radius: 4; color: session && session.connected ? Theme.success : Theme.danger }
            Text { color: session && session.connected ? Theme.success : Theme.danger; font.pixelSize: 11
                text: session && session.connected ? "Daemon connected (port " + session.port + ")" : "Daemon disconnected" }
        },

        Text { text: "Uptime: " + (formatUptime ? formatUptime(statsDialog.statsData.uptime_secs || 0) : ""); color: Theme.textDim; font.pixelSize: 10 }
    ]

    buttons: [
        Rectangle { width: 70; height: 26; radius: 3; color: statsCloseArea.containsMouse ? Theme.border : Theme.surfaceAlt
            Text { anchors.centerIn: parent; text: qsTr("Close"); color: Theme.text; font.pixelSize: 12 }
            MouseArea { id: statsCloseArea; anchors.fill: parent; hoverEnabled: true; onClicked: statsDialog.close() }
        }
    ]
}
