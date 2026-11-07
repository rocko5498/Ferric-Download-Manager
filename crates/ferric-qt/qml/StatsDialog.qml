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
        Row { spacing: Theme.sp2
            Rectangle { width: 50; height: 22; radius: 3; color: Theme.surfaceAlt
                Text { anchors.centerIn: parent; text: qsTr("Refresh"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { anchors.fill: parent; onClicked: { if (loadStats) loadStats(); } }
            }
        },

        Text { text: qsTr("Session"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true },

        Grid { columns: 2; spacing: Theme.sp2; width: parent.width
            Text { text: qsTr("Active downloads:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 140 }
            Text { text: (statsDialog.statsData.active_downloads || 0).toString(); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
            Text { text: qsTr("Total downloads:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 140 }
            Text { text: (statsDialog.statsData.total_downloads || 0).toString(); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
        },

        Text { text: qsTr("Speed"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true },

        Grid { columns: 2; spacing: Theme.sp2; width: parent.width
            Text { text: qsTr("Download speed:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 140 }
            Text { text: formatBps ? formatBps(statsDialog.statsData.download_bps || 0) : ""; color: Theme.success; font.pixelSize: Theme.fontSizeSmall; font.bold: true }
        },

        Text { text: qsTr("Connection"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true },

        Row { spacing: Theme.sp2; height: 22
            Rectangle { width: 8; height: 8; radius: 4; color: session && session.connected ? Theme.success : Theme.danger }
            Text { color: session && session.connected ? Theme.success : Theme.danger; font.pixelSize: Theme.fontSizeSmall
                text: session && session.connected ? "Daemon connected (port " + session.port + ")" : "Daemon disconnected" }
        },

        Text { text: "Uptime: " + (formatUptime ? formatUptime(statsDialog.statsData.uptime_secs || 0) : ""); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
    ]

    buttons: [
        Rectangle { width: Theme.btnM; height: Theme.controlH; radius: Theme.radius; color: statsCloseArea.containsMouse ? Theme.border : Theme.surfaceAlt
            Text { anchors.centerIn: parent; text: qsTr("Close"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
            MouseArea { id: statsCloseArea; anchors.fill: parent; hoverEnabled: true; onClicked: statsDialog.close() }
        }
    ]
}
