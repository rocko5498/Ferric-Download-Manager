import QtQuick

// FQT-046: Site Rules dialog \u2014 extracted from main.qml
AppDialog {
    id: siteRuleDialog

    property var bus
    property var requestBus
    property var siteRules: []
    property string ruleDialogMode: "add"
    property var editingRule: null
    property var loadSiteRules
    property var doAddSiteRule
    property var doEditSiteRule
    property var doDeleteSiteRule
    property var showError
    property var resetMode

    title: qsTr("Site Rules")
    cardMaxHeight: 580

    function prepareAdd() {
        rulePatternInput2.text = "";
        ruleTypeCombo2.currentIndex = 0;
        ruleValueInput2.text = "";
    }

    function prepareEdit(rule) {
        rulePatternInput2.text = rule.pattern || "";
        var types = ["capture", "skip", "max_connections", "max_speed_bps", "always_ask"];
        ruleTypeCombo2.currentIndex = types.indexOf(rule.rule_type) >= 0 ? types.indexOf(rule.rule_type) : 0;
        ruleValueInput2.text = rule.value || "";
    }

    contentItem: [
        Row {
            width: parent.width; spacing: 8
            Item { width: parent.width - 170; height: 1 }
            Rectangle {
                width: 60; height: 22; radius: 3; color: siteRuleAddArea.containsMouse ? Theme.accent : Theme.accentDark
                Text { anchors.centerIn: parent; text: qsTr("+ Add"); color: "white"; font.pixelSize: 10 }
                MouseArea { id: siteRuleAddArea; anchors.fill: parent; hoverEnabled: true; onClicked: if (doAddSiteRule) doAddSiteRule() }
            }
            Rectangle {
                width: 60; height: 22; radius: 3; color: siteRuleRefreshArea.containsMouse ? Theme.surfaceAlt : "transparent"
                Text { anchors.centerIn: parent; text: qsTr("Refresh"); color: Theme.textDim; font.pixelSize: 10 }
                MouseArea { id: siteRuleRefreshArea; anchors.fill: parent; hoverEnabled: true; onClicked: if (loadSiteRules) loadSiteRules() }
            }
        },

        Rectangle { width: parent.width; height: 1; color: Theme.surfaceAlt },

        // Table header
        Row {
            width: parent.width; height: 20; spacing: 0
            Text { width: 40; text: qsTr("#"); color: Theme.textDim; font.pixelSize: 10 }
            Text { width: 150; text: qsTr("Pattern"); color: Theme.textDim; font.pixelSize: 10 }
            Text { width: 90; text: qsTr("Type"); color: Theme.textDim; font.pixelSize: 10 }
            Text { width: 110; text: qsTr("Value"); color: Theme.textDim; font.pixelSize: 10 }
            Text { width: 50; text: qsTr("Pri"); color: Theme.textDim; font.pixelSize: 10 }
            Text { width: 50; text: qsTr(""); color: Theme.textDim; font.pixelSize: 10 }
        },

        // Rules list
        ListView {
            width: parent.width
            height: 180
            clip: true
            cacheBuffer: 200
            model: siteRuleDialog.siteRules
            delegate: Rectangle {
                width: ListView.view.width
                height: 22
                color: Theme.bg
                Row {
                    anchors.fill: parent
                    Text { width: 40; text: (index + 1); color: Theme.textDim; font.pixelSize: 10 }
                    Text { width: 150; text: modelData.pattern; color: Theme.text; font.pixelSize: 10; elide: Text.ElideRight }
                    Text { width: 90; text: modelData.rule_type; color: Theme.textDim; font.pixelSize: 10 }
                    Text { width: 110; text: modelData.value || "-"; color: Theme.textDim; font.pixelSize: 10; elide: Text.ElideRight }
                    Text { width: 50; text: modelData.priority; color: Theme.textDim; font.pixelSize: 10 }
                    Row {
                        width: 50; spacing: 2
                        Rectangle {
                            width: 20; height: 16; radius: 2; color: "transparent"
                            Text { anchors.centerIn: parent; text: qsTr("\u270E"); color: Theme.textDim; font.pixelSize: 10 }
                            MouseArea { anchors.fill: parent; onClicked: if (doEditSiteRule) doEditSiteRule(modelData) }
                        }
                        Rectangle {
                            width: 20; height: 16; radius: 2; color: "transparent"
                            Text { anchors.centerIn: parent; text: qsTr("\u2715"); color: Theme.danger; font.pixelSize: 10 }
                            MouseArea { anchors.fill: parent; onClicked: if (doDeleteSiteRule) doDeleteSiteRule(modelData.id) }
                        }
                    }
                }
            }

            Text {
                visible: siteRuleDialog.siteRules.length === 0
                anchors.centerIn: parent
                text: qsTr("No site rules")
                color: Theme.textDim; font.pixelSize: 12
            }
        },

        Rectangle { width: parent.width; height: 1; color: Theme.surfaceAlt },

        Text { text: siteRuleDialog.ruleDialogMode === "edit" ? "Edit Rule" : "New Rule"; color: Theme.accent; font.pixelSize: 11; font.bold: true },

        Row { spacing: 8; width: parent.width
            Column { width: 200; spacing: 3
                Text { text: qsTr("Pattern (*.example.com/*)"); color: Theme.textDim; font.pixelSize: 9 }
                Rectangle {
                    width: parent.width; height: 24; color: Theme.bg
                    border.color: rulePatternInput2.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: 3
                    TextInput {
                        clip: true
                        id: rulePatternInput2
                        anchors { fill: parent; margins: 4 }
                        color: Theme.text; font.pixelSize: 11
                    }
                }
            }
            Column { width: 100; spacing: 3
                Text { text: qsTr("Type"); color: Theme.textDim; font.pixelSize: 9 }
                Rectangle {
                    width: parent.width; height: 24; color: Theme.bg
                    border.color: Theme.surfaceAlt; radius: 3
                    Item {
                        id: ruleTypeCombo2
                        property var options: ["capture", "skip", "max_connections", "max_speed_bps", "always_ask"]
                        property int currentIndex: 0
                        anchors { fill: parent; margins: 2 }
                        Text { anchors.centerIn: parent; text: ruleTypeCombo2.options[ruleTypeCombo2.currentIndex]; color: Theme.text; font.pixelSize: 10 }
                        MouseArea { anchors.fill: parent; onClicked: ruleTypeCombo2.currentIndex = (ruleTypeCombo2.currentIndex + 1) % ruleTypeCombo2.options.length }
                    }
                }
            }
            Column { width: 120; spacing: 3
                Text { text: qsTr("Value"); color: Theme.textDim; font.pixelSize: 9 }
                Rectangle {
                    width: parent.width; height: 24; color: Theme.bg
                    border.color: ruleValueInput2.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: 3
                    TextInput {
                        clip: true
                        id: ruleValueInput2
                        anchors { fill: parent; margins: 4 }
                        color: Theme.text; font.pixelSize: 11
                    }
                }
            }
        },

        Row {
            anchors.right: parent.right; spacing: 8
            Rectangle {
                width: 50; height: 22; radius: 3; color: Theme.accent
                Text { anchors.centerIn: parent; text: qsTr("Save"); color: "white"; font.pixelSize: 10 }
                MouseArea { anchors.fill: parent; onClicked: {
                    var pattern = rulePatternInput2.text.trim();
                    var types = ["capture", "skip", "max_connections", "max_speed_bps", "always_ask"];
                    var ruleType = types[ruleTypeCombo2.currentIndex] || "capture";
                    var value = ruleValueInput2.text.trim();
                    if (pattern === "") { showError("Pattern required"); return; }
                    if (!requestBus) return;
                    var op = siteRuleDialog.ruleDialogMode === "edit" && siteRuleDialog.editingRule ? "update_site_rule" : "add_site_rule";
                    var args = { pattern: pattern, rule_type: ruleType, value: value };
                    if (op === "update_site_rule") args.id = siteRuleDialog.editingRule.id;
                    requestBus(op, args, function(result) {
                        var res = JSON.parse(result);
                        if (res.error) { showError(res.error); return; }
                        if (loadSiteRules) loadSiteRules();
                        rulePatternInput2.text = ""; ruleValueInput2.text = ""; ruleTypeCombo2.currentIndex = 0;
                        ruleDialogMode = "add"; editingRule = null;
                        if (resetMode) resetMode();
                    });
                }}
            }
        }
    ]

    buttons: []
}
