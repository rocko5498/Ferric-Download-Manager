import QtQuick
import QtQuick.Controls as QQC

// Overview workspace \u2014 extracted from main.qml (Phase 2 split #1)
// Stats cards, privacy card, network mode strip, recent transfers, quick add
Rectangle {
    id: overview

    color: Theme.bg

    // Data bindings (passed from main.qml root)
    property var stats: ({})
    property var anonymityScore: null
    property string anonymityGrade: ""
    property bool killSwitchActive: false
    property var privacyDashboard: ({})
    property string privacyRecommendation: ""
    property var recentTransfers: []
    property var prefsData: ({})
    property bool torrentsVisible: false
    readonly property int cardCount: torrentsVisible ? 5 : 2

    // Functions passed from root
    property var onRefresh: function() {}
    property var onLoadPreferences: function() {}
    property var onAddUrl: function(url, filename, savePath, cookies) {}
    property var onSelectTransfer: function(id, status, name, savePath, url) {}

    function formatBps(bps) {
        if (!bps || bps === 0) return "0 B/s";
        if (bps < 1024) return bps.toFixed(0) + " B/s";
        if (bps < 1048576) return (bps / 1024).toFixed(1) + " KB/s";
        if (bps < 1073741824) return (bps / 1048576).toFixed(1) + " MB/s";
        return (bps / 1073741824).toFixed(2) + " GB/s";
    }

    function formatBytes(bytes) {
        if (!bytes || bytes === 0) return "0 B";
        if (bytes < 1024) return bytes + " B";
        if (bytes < 1048576) return (bytes / 1024).toFixed(1) + " KB";
        if (bytes < 1073741824) return (bytes / 1048576).toFixed(1) + " MB";
        return (bytes / 1073741824).toFixed(2) + " GB";
    }

    function formatAnonymityScore() {
        if (anonymityScore === null || anonymityScore === undefined)
            return qsTr("N/A");
        return anonymityScore + "/100";
    }

    function gradeBadgeColor() {
        switch (anonymityGrade) {
        case "A": return Theme.success;
        case "B": return Theme.success;
        case "C": return Theme.warning;
        case "D": return Theme.warning;
        case "F": return Theme.danger;
        default:  return Theme.textDim;
        }
    }

    Column {
        anchors.fill: parent
        anchors.margins: Theme.sp3
        spacing: Theme.sp2

        Row { spacing: Theme.sp3; width: parent.width
            Text { text: qsTr("Overview"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal; font.bold: true }
            Rectangle { width: 50; height: 22; radius: 3; color: refreshArea.containsMouse ? Theme.border : Theme.surfaceAlt
                Text { anchors.centerIn: parent; text: qsTr("Refresh"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { id: refreshArea; anchors.fill: parent; hoverEnabled: true; onClicked: overview.onRefresh() }
            }
        }

        // Stats cards
        Row { spacing: Theme.sp2; width: parent.width
            Rectangle { width: (parent.width - 10 * (overview.cardCount - 1)) / overview.cardCount; height: 60; radius: 4; color: Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                Column { anchors.centerIn: parent; spacing: 2
                    Text { text: qsTr("Downloads"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; anchors.horizontalCenter: parent.horizontalCenter }
                    Text { text: (overview.stats.active_downloads || 0) + " active"; color: Theme.success; font.pixelSize: Theme.fontSizeNormal; font.bold: true; anchors.horizontalCenter: parent.horizontalCenter }
                    Text { text: (overview.stats.total_downloads || 0) + " total"; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; anchors.horizontalCenter: parent.horizontalCenter }
                }
            }
            Rectangle { visible: overview.torrentsVisible; width: (parent.width - 10 * (overview.cardCount - 1)) / overview.cardCount; height: 60; radius: 4; color: Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                Column { anchors.centerIn: parent; spacing: 2
                    Text { text: qsTr("Torrents"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; anchors.horizontalCenter: parent.horizontalCenter }
                    Text { text: (overview.stats.active_torrents || 0) + " active"; color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true; anchors.horizontalCenter: parent.horizontalCenter }
                    Text { text: (overview.stats.total_torrents || 0) + " total"; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; anchors.horizontalCenter: parent.horizontalCenter }
                }
            }
            Rectangle { width: (parent.width - 10 * (overview.cardCount - 1)) / overview.cardCount; height: 60; radius: 4; color: Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                Column { anchors.centerIn: parent; spacing: 2
                    Text { text: qsTr("Download speed"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; anchors.horizontalCenter: parent.horizontalCenter }
                    Text { text: formatBps(overview.stats.download_bps || 0); color: Theme.text; font.pixelSize: Theme.fontSizeNormal; font.bold: true; anchors.horizontalCenter: parent.horizontalCenter }
                }
            }
            Rectangle { visible: overview.torrentsVisible; width: (parent.width - 10 * (overview.cardCount - 1)) / overview.cardCount; height: 60; radius: 4; color: Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                Column { anchors.centerIn: parent; spacing: 2
                    Text { text: qsTr("Upload speed"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; anchors.horizontalCenter: parent.horizontalCenter }
                    Text { text: formatBps(overview.stats.upload_bps || 0); color: Theme.text; font.pixelSize: Theme.fontSizeNormal; font.bold: true; anchors.horizontalCenter: parent.horizontalCenter }
                }
            }
            Rectangle { visible: overview.torrentsVisible; width: (parent.width - 10 * (overview.cardCount - 1)) / overview.cardCount; height: 60; radius: 4; color: Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                Column { anchors.centerIn: parent; spacing: 2
                    Text { text: qsTr("Privacy"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; anchors.horizontalCenter: parent.horizontalCenter }
                    Row {
                        spacing: Theme.sp1
                        anchors.horizontalCenter: parent.horizontalCenter
                        Text {
                            text: formatAnonymityScore()
                            color: Theme.text; font.pixelSize: Theme.fontSizeNormal; font.bold: true
                            anchors.verticalCenter: parent.verticalCenter
                        }
                        Rectangle {
                            visible: overview.anonymityGrade.length > 0
                            width: gradeBadgeText.implicitWidth + 12
                            height: 16
                            radius: 3
                            color: gradeBadgeColor()
                            border.color: Theme.surfaceAlt
                            border.width: 1
                            anchors.verticalCenter: parent.verticalCenter
                            Text {
                                id: gradeBadgeText
                                anchors.centerIn: parent
                                text: overview.anonymityGrade
                                color: Theme.bg
                                font.pixelSize: Theme.fontSizeSmall
                                font.bold: true
                            }
                        }
                    }
                    Text {
                        text: overview.killSwitchActive ? qsTr("Kill switch ON") : qsTr("Kill switch off")
                        color: overview.killSwitchActive ? Theme.danger : Theme.textDim
                        font.pixelSize: Theme.fontSizeSmall
                        anchors.horizontalCenter: parent.horizontalCenter
                    }
                }
            }
        }

        // Network mode strip
        Rectangle {
            visible: overview.torrentsVisible
            width: parent.width; height: 32; radius: 4
            color: Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
            Row {
                anchors.verticalCenter: parent.verticalCenter
                anchors.left: parent.left
                anchors.leftMargin: Theme.sp2
                spacing: Theme.sp2
                Text { text: qsTr("Network mode"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Text {
                    font.pixelSize: Theme.fontSizeNormal; font.bold: true
                    color: {
                        var m = overview.prefsData["product_mode"] || "balanced"
                        if (m === "strict") return Theme.warning
                        if (m === "open") return Theme.accent
                        return Theme.success
                    }
                    text: {
                        var m = overview.prefsData["product_mode"] || "balanced"
                        if (m === "strict") return qsTr("Strict")
                        if (m === "open") return qsTr("Open")
                        return qsTr("Balanced")
                    }
                }
                Text {
                    color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
                    text: {
                        var m = overview.prefsData["product_mode"] || "balanced"
                        if (m === "strict")
                            return qsTr("\u2014 no DHT/PEX/LSD \u00b7 proxy required \u00b7 no inbound listen")
                        if (m === "open")
                            return qsTr("\u2014 full discovery \u00b7 inbound listen \u00b7 default seeding")
                        return qsTr("\u2014 DHT+PEX \u00b7 outbound-only \u00b7 proxy optional")
                    }
                }
            }
            MouseArea {
                anchors.fill: parent
                onClicked: overview.onLoadPreferences()
            }
        }

        // Privacy dashboard strip
        Rectangle {
            visible: overview.torrentsVisible
            width: parent.width; height: 28; radius: 4
            color: Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
            Row {
                anchors { fill: parent; leftMargin: Theme.sp2; rightMargin: Theme.sp2 }
                spacing: Theme.sp3
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: qsTr("%1 hosts \u00b7 %2 events").arg(overview.privacyDashboard.unique_hosts || 0).arg(overview.privacyDashboard.total_events || 0)
                    color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    width: parent.width - 160
                    elide: Text.ElideRight
                    text: overview.privacyRecommendation.length > 0 ? overview.privacyRecommendation : qsTr("No privacy recommendations")
                    color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                }
            }
        }

        // Recent transfers
        Text { text: qsTr("Recent transfers"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

        Flickable { width: parent.width; height: parent.height - (overview.torrentsVisible ? 250 : 170); clip: true; contentHeight: recentCol.height
            QQC.ScrollBar.vertical: QQC.ScrollBar {}
            Column { id: recentCol; width: parent.width; spacing: Theme.sp1
                Repeater {
                    model: overview.recentTransfers
                    Item { height: 24; width: recentCol.width
                        Row { anchors.fill: parent; spacing: Theme.sp2
                            Rectangle { width: 8; height: 8; radius: 4; anchors.verticalCenter: parent.verticalCenter
                                color: modelData.status === "completed" ? Theme.success : modelData.status === "downloading" ? Theme.accent : modelData.status === "error" ? Theme.danger : Theme.textDim
                            }
                            Text { text: modelData.filename || modelData.id; textFormat: Text.PlainText; color: Theme.text; font.pixelSize: Theme.fontSizeSmall; width: parent.width - 160; elide: Text.ElideRight; anchors.verticalCenter: parent.verticalCenter }
                            Text { text: modelData.status || "?"; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 89; anchors.verticalCenter: parent.verticalCenter }
                            Text { text: overview.formatBytes(modelData.total_size || 0); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 89; horizontalAlignment: Text.AlignRight; anchors.verticalCenter: parent.verticalCenter }
                        }
                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onClicked: overview.onSelectTransfer(modelData.id, modelData.status, modelData.filename, modelData.save_path, modelData.url)
                        }
                    }
                }
                Text { text: overview.recentTransfers.length === 0 ? "No recent transfers" : ""; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            }
        }

        // Quick add
        Row { spacing: Theme.sp2; width: parent.width
            Rectangle {
                width: parent.width - 80; height: 26; radius: 3; color: Theme.bg
                border.color: overviewUrlInput.activeFocus ? Theme.accent : Theme.surfaceAlt
                TextInput { id: overviewUrlInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                    clip: true
                    Text { visible: parent.text.length === 0; text: qsTr("Paste URL to quick add..."); color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal } }
            }
            Rectangle { width: 70; height: 26; radius: 3; color: overviewAddArea.containsMouse ? Theme.border : Theme.surfaceAlt
                Text { anchors.centerIn: parent; text: qsTr("Add"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { id: overviewAddArea; anchors.fill: parent; hoverEnabled: true
                    onClicked: {
                        if (overviewUrlInput.text.length > 0) {
                            overview.onAddUrl(overviewUrlInput.text, "", "", "");
                            overviewUrlInput.text = "";
                        }
                    }
                }
            }
        }
    }
}
