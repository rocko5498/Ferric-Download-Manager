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

    Flickable {
        id: scroller
        anchors.fill: parent
        contentWidth: width
        contentHeight: mainCol.height + 2 * Theme.sp4
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        QQC.ScrollBar.vertical: QQC.ScrollBar {}

        Column {
            id: mainCol
            x: Theme.sp4; y: Theme.sp4
            width: scroller.width - 2 * Theme.sp4
            spacing: Theme.sp3

            Column {
                spacing: 2
                Text { text: qsTr("RSS"); color: Theme.text; font.pixelSize: Theme.fontSizeLarge; font.weight: Font.Bold }
                Text { text: qsTr("Follow a feed and let Ferric download new items that match your rules."); color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal }
            }

            Card {
                heading: qsTr("Feeds")
                Text { visible: rssWorkspace.rssFeeds.length === 0; text: qsTr("No feeds yet. Add one to start following it."); color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal }
                Repeater {
                    model: rssWorkspace.rssFeeds
                    Row {
                        width: parent.width; height: Theme.rowH; spacing: Theme.sp2
                        Text { anchors.verticalCenter: parent.verticalCenter; width: parent.width - pollText.width - delFeed.width - 2 * Theme.sp2
                            text: modelData.url || "-"; color: Theme.text; font.pixelSize: Theme.fontSizeNormal; elide: Text.ElideRight }
                        Text { id: pollText; anchors.verticalCenter: parent.verticalCenter; width: Theme.btnM + Theme.sp3
                            text: qsTr("every %1 s").arg(modelData.poll_secs || "-"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                        Rectangle {
                            id: delFeed
                            anchors.verticalCenter: parent.verticalCenter
                            width: Theme.btnM - Theme.sp3; height: Theme.sp4 + Theme.sp1; radius: Theme.radius
                            color: delFeedArea.containsMouse ? Theme.dangerSoft : "transparent"
                            Text { anchors.centerIn: parent; text: qsTr("Remove"); color: Theme.danger; font.pixelSize: Theme.fontSizeSmall; font.weight: Font.DemiBold }
                            MouseArea { id: delFeedArea; anchors.fill: parent; hoverEnabled: true; onClicked: rssWorkspace.onDeleteFeed(modelData.id) }
                        }
                    }
                }
                Row {
                    spacing: Theme.sp2
                    PillButton { label: qsTr("Add feed"); onClicked: rssWorkspace.onAddFeed() }
                    PillButton { label: qsTr("Refresh"); primary: false; onClicked: rssWorkspace.onRefreshFeeds() }
                }
            }

            Card {
                heading: qsTr("Auto-download rules")
                Text { visible: rssWorkspace.rssRules.length === 0; text: qsTr("No rules yet. A rule downloads feed items whose title matches a pattern."); color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal }
                Repeater {
                    model: rssWorkspace.rssRules
                    Row {
                        width: parent.width; height: Theme.sp5 + Theme.sp3; spacing: Theme.sp2
                        Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 8; height: 8; radius: 4; color: modelData.enabled ? Theme.success : Theme.textDim }
                        Column {
                            anchors.verticalCenter: parent.verticalCenter
                            width: parent.width - 8 - delRule.width - 2 * Theme.sp2
                            Text { text: modelData.name + " — " + modelData.match_pattern; color: Theme.text; font.pixelSize: Theme.fontSizeNormal; elide: Text.ElideRight; width: parent.width }
                            Text {
                                width: parent.width; elide: Text.ElideRight; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
                                text: {
                                    var feedUrl = modelData.feed_id;
                                    for (var i = 0; i < rssWorkspace.rssFeeds.length; i++)
                                        if (rssWorkspace.rssFeeds[i].id === modelData.feed_id) { feedUrl = rssWorkspace.rssFeeds[i].url; break; }
                                    return qsTr("Feed: ") + feedUrl + "  ·  " + qsTr("Action: ") + modelData.action;
                                }
                            }
                        }
                        Rectangle {
                            id: delRule
                            anchors.verticalCenter: parent.verticalCenter
                            width: Theme.btnM - Theme.sp3; height: Theme.sp4 + Theme.sp1; radius: Theme.radius
                            color: delRuleArea.containsMouse ? Theme.dangerSoft : "transparent"
                            Text { anchors.centerIn: parent; text: qsTr("Remove"); color: Theme.danger; font.pixelSize: Theme.fontSizeSmall; font.weight: Font.DemiBold }
                            MouseArea { id: delRuleArea; anchors.fill: parent; hoverEnabled: true; onClicked: rssWorkspace.onDeleteRule(modelData.id) }
                        }
                    }
                }
                PillButton { label: qsTr("Add rule"); onClicked: rssWorkspace.onAddRule() }
            }
        }
    }
}
