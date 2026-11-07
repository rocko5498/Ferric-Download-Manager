import QtQuick

// Add RSS Rule dialog \u2014 extracted from main.qml (Phase 2 split #11)
AppDialog {
    id: addRssRuleDialog

    property var feedModel: null
    property var onAdd: function(feedId, name, pattern) {}
    property var showError: function(msg) {}

    title: qsTr("Add Auto-download Rule")

    contentItem: [
        Column { width: parent.width; spacing: Theme.sp1
            Text { text: qsTr("Feed"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            Rectangle { width: parent.width; height: Theme.controlH; color: Theme.bg; border.color: Theme.surfaceAlt; radius: Theme.radius
                Item { id: rssRuleFeedCombo; property int currentIndex: 0; anchors.fill: parent
                    Text { anchors { fill: parent; leftMargin: Theme.sp1; rightMargin: Theme.sp1 } verticalAlignment: Text.AlignVCenter; elide: Text.ElideRight; color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                        text: addRssRuleDialog.feedModel && addRssRuleDialog.feedModel.count > 0 ? addRssRuleDialog.feedModel.get(Math.min(rssRuleFeedCombo.currentIndex, addRssRuleDialog.feedModel.count - 1)).text : qsTr("No feeds") }
                    MouseArea { anchors.fill: parent; onClicked: if (addRssRuleDialog.feedModel && addRssRuleDialog.feedModel.count > 0) rssRuleFeedCombo.currentIndex = (rssRuleFeedCombo.currentIndex + 1) % addRssRuleDialog.feedModel.count } } }
        },

        Column { width: parent.width; spacing: Theme.sp1
            Text { text: qsTr("Rule name"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            Rectangle { width: parent.width; height: Theme.controlH; color: Theme.bg; border.color: rssRuleNameInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                TextInput { id: rssRuleNameInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal } }
        clip: true
        },

        Column { width: parent.width; spacing: Theme.sp1
            Text { text: qsTr("Match pattern (contains)"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            Rectangle { width: parent.width; height: Theme.controlH; color: Theme.bg; border.color: rssRulePatternInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                TextInput { id: rssRulePatternInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                    Text { visible: parent.text.length === 0; text: qsTr("e.g. .mp4 or 720p"); color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal; anchors.verticalCenter: parent.verticalCenter } } }
        }
    ]

    buttons: [
        Rectangle { width: Theme.btnM; height: Theme.controlH; radius: Theme.radius; color: Theme.bg
            Text { anchors.centerIn: parent; text: qsTr("Cancel"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            MouseArea { anchors.fill: parent; onClicked: addRssRuleDialog.close() }
        },
        Rectangle { width: Theme.btnM; height: Theme.controlH; radius: Theme.radius; color: Theme.accent
            Text { anchors.centerIn: parent; text: qsTr("Add"); color: "white"; font.pixelSize: Theme.fontSizeSmall }
            MouseArea {
                anchors.fill: parent
                onClicked: {
                    if (!addRssRuleDialog.feedModel || addRssRuleDialog.feedModel.count === 0) return;
                    var feedId = addRssRuleDialog.feedModel.get(rssRuleFeedCombo.currentIndex).feedId;
                    addRssRuleDialog.onAdd(feedId, rssRuleNameInput.text, rssRulePatternInput.text);
                }
            }
        }
    ]
}
