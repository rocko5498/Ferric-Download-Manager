import QtQuick
import QtQuick.Controls as QQC

// RSS workspace \u2014 extracted from main.qml (Phase 2 split #17)
Rectangle {
    id: rssWorkspace

    color: Theme.bg

    property var rssFeeds: []
    property var rssRules: []
    property var bus: null
    property var onAddFeed: function() {}
    property var onRefreshFeeds: function() {}
    property var onDeleteFeed: function(id) {}
    property var onAddRule: function() {}
    property var onDeleteRule: function(id) {}
    property var showError: function(msg) {}

    Column {
        anchors.fill: parent
        anchors.margins: Theme.sp3
        spacing: Theme.sp2

        Row {
            spacing: Theme.sp2
            Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("RSS Feeds"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal; font.bold: true }
            Rectangle {
                width: 60; height: Theme.rowH; radius: Theme.radius; color: rssAddArea.containsMouse ? Theme.accentDark : Theme.accent
                Text { anchors.centerIn: parent; text: qsTr("+ Add"); color: "white"; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { id: rssAddArea; anchors.fill: parent; hoverEnabled: true; onClicked: rssWorkspace.onAddFeed() }
            }
            Rectangle {
                width: 60; height: Theme.rowH; radius: Theme.radius; color: rssRefreshArea.containsMouse ? Theme.surfaceAlt : "transparent"
                Text { anchors.centerIn: parent; text: qsTr("Refresh"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { id: rssRefreshArea; anchors.fill: parent; hoverEnabled: true; onClicked: rssWorkspace.onRefreshFeeds() }
            }
        }

        // Feeds list header
        Row {
            width: parent.width; height: Theme.rowH; spacing: 0
            Text { anchors.verticalCenter: parent.verticalCenter; width: 30; text: qsTr("#"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            Text { anchors.verticalCenter: parent.verticalCenter; width: 250; text: qsTr("URL"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            Text { anchors.verticalCenter: parent.verticalCenter; width: 80; text: qsTr("Poll (s)"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            Text { anchors.verticalCenter: parent.verticalCenter; width: 50; text: qsTr("Action"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
        }

        Rectangle { width: parent.width; height: 1; color: Theme.surfaceAlt }

        // Feeds list
        ListView {
            width: parent.width
            height: Math.max(60, (parent.height - 126) * 0.6)
            clip: true
            cacheBuffer: 200
            model: rssWorkspace.rssFeeds
            QQC.ScrollBar.vertical: QQC.ScrollBar {}
            delegate: Rectangle {
                width: ListView.view.width
                height: Theme.rowH
                color: Theme.bg
                Row {
                    anchors.fill: parent
                    Text { anchors.verticalCenter: parent.verticalCenter; width: 30; text: (index + 1); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: 250; text: modelData.url || "-"; color: Theme.text; font.pixelSize: Theme.fontSizeSmall; elide: Text.ElideRight }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: 80; text: modelData.poll_secs || "-"; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    Rectangle {
                        width: 45; height: Theme.rowH; radius: Theme.radius; color: Theme.surfaceAlt
                        Text { anchors.centerIn: parent; text: qsTr("Delete"); color: Theme.danger; font.pixelSize: Theme.fontSizeSmall }
                        MouseArea { anchors.fill: parent; onClicked: rssWorkspace.onDeleteFeed(modelData.id) }
                    }
                }
            }

            Text {
                visible: rssWorkspace.rssFeeds.length === 0
                anchors.centerIn: parent
                text: qsTr("No RSS feeds \u2014 click + Add to subscribe")
                color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal
            }
        }

        // RSS auto-download rules
        Rectangle { width: parent.width; height: 1; color: Theme.surfaceAlt }

        Row {
            spacing: Theme.sp2
            Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Auto-download Rules"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal; font.bold: true }
            Rectangle {
                width: 60; height: Theme.rowH; radius: Theme.radius; color: rssRuleAddArea.containsMouse ? Theme.accentDark : Theme.accent
                Text { anchors.centerIn: parent; text: qsTr("+ Add"); color: "white"; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { id: rssRuleAddArea; anchors.fill: parent; hoverEnabled: true; onClicked: rssWorkspace.onAddRule() }
            }
        }

        Flickable { width: parent.width; height: Math.max(40, (parent.height - 126) * 0.4); clip: true; contentHeight: rssRulesCol.height
            QQC.ScrollBar.vertical: QQC.ScrollBar {}
            Column { id: rssRulesCol; width: parent.width; spacing: Theme.sp1
                Repeater {
                    model: rssWorkspace.rssRules
                    Row { spacing: Theme.sp1; height: Theme.rowH; width: rssRulesCol.width
                        Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 8; height: 8; radius: 4; color: modelData.enabled ? Theme.success : Theme.danger }
                        Column { width: rssRulesCol.width - 100
                            Text { text: modelData.name + " \u2014 " + modelData.match_pattern; color: Theme.text; font.pixelSize: Theme.fontSizeSmall; elide: Text.ElideRight; width: parent.width }
                            Text {
                                text: {
                                    var feedUrl = modelData.feed_id;
                                    for (var i = 0; i < rssWorkspace.rssFeeds.length; i++)
                                        if (rssWorkspace.rssFeeds[i].id === modelData.feed_id) { feedUrl = rssWorkspace.rssFeeds[i].url; break; }
                                    return "Feed: " + feedUrl + " | Action: " + modelData.action;
                                }
                                color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
                            }
                        }
                        Rectangle {
                            anchors.verticalCenter: parent.verticalCenter
                            width: 45; height: Theme.rowH; radius: Theme.radius; color: Theme.surfaceAlt
                            Text { anchors.centerIn: parent; text: qsTr("Delete"); color: Theme.danger; font.pixelSize: Theme.fontSizeSmall }
                            MouseArea { anchors.fill: parent; onClicked: rssWorkspace.onDeleteRule(modelData.id) }
                        }
                    }
                }
                Text { visible: rssWorkspace.rssRules.length === 0; text: qsTr("No rules \u2014 add a rule to auto-download matching items"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            }
        }
    }
}
