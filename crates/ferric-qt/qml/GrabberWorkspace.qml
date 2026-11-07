import QtQuick

// Link Grabber workspace \u2014 extracted from main.qml (Phase 2 split #2)
Rectangle {
    id: grabber

    color: Theme.bg

    // Data bindings
    property string grabUrl: ""
    property string grabExtensions: ""
    property int grabDepth: 0
    property string grabExclude: ""
    property string grabInclude: ""
    property bool grabStayBelow: false
    property int grabExternal: 0
    property var projects: []
    property var onProjectSave: function(name, asTemplate) {}
    property var onProjectOpen: function(id) {}
    property var onProjectDelete: function(id) {}
    property var onProjectUpdate: function() {}
    property bool grabDownloadAll: false

    // Fill the settings fields from a saved project or template.
    function applyConfig(cfg) {
        if (!cfg) return;
        grabUrlInput.text = cfg.url || "";
        grabExtInput.text = cfg.extensions || "";
        grabDepthInput.text = String(cfg.depth || 0);
        grabExternalInput.text = String(cfg.external_levels || 0);
        grabIncludeInput.text = cfg.include || "";
        grabExcludeInput.text = cfg.exclude || "";
        grabber.grabStayBelow = cfg.stay_below === true;
    }
    property var grabResults: []
    property var grabSelected: []
    property bool grabLoading: false

    // Functions
    property var onGrab: function() {}
    property var onToggleResult: function(idx) {}
    property var onAddSelected: function() {}

    Column {
        anchors.fill: parent
        anchors.margins: Theme.sp3
        spacing: Theme.sp2

        Text { text: qsTr("Link Grabber"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

        // URL input
        Row {
            width: parent.width; spacing: Theme.sp1
            Rectangle {
                width: parent.width - 160; height: 28; color: Theme.bg
                border.color: grabUrlInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: 3
                TextInput {
                    clip: true
                    id: grabUrlInput
                    anchors { fill: parent; margins: Theme.sp1 }
                    color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                    Text { visible: parent.text.length === 0; text: qsTr("https://example.com/page"); color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal }
                    // The typed URL must reach the root state (it used to stay in this
                    // field, so Grab always answered "URL required").
                    onTextChanged: grabber.grabUrl = text
                    onAccepted: grabber.onGrab()
                }
            }
            Rectangle {
                width: 70; height: 28; radius: 3
                color: grabBtnArea.containsMouse ? Theme.accent : Theme.accentDark
                Text { anchors.centerIn: parent; text: grabber.grabLoading ? "..." : "Grab"; color: "white"; font.pixelSize: Theme.fontSizeNormal }
                MouseArea { id: grabBtnArea; anchors.fill: parent; hoverEnabled: true; onClicked: grabber.onGrab() }
            }
        }

        // Options row
        Row {
            spacing: Theme.sp3
            Row {
                spacing: Theme.sp1
                Text { text: qsTr("Extensions:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Rectangle {
                    width: 140; height: 24; color: Theme.bg
                    border.color: grabExtInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: 3
                    TextInput {
                        clip: true
                        id: grabExtInput
                        anchors { fill: parent; margins: Theme.sp1 }
                        color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                        Text { visible: parent.text.length === 0; text: qsTr("mp4,zip,pdf"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                        onTextChanged: grabber.grabExtensions = text
                    }
                }
            }
            Row {
                spacing: Theme.sp1
                Text { text: qsTr("Depth:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Rectangle {
                    width: 40; height: 24; color: Theme.bg
                    border.color: grabDepthInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: 3
                    TextInput {
                        clip: true
                        id: grabDepthInput
                        anchors { fill: parent; margins: Theme.sp1 }
                        color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                        inputMethodHints: Qt.ImhDigitsOnly
                        text: qsTr("0")
                        onTextChanged: grabber.grabDepth = Math.min(5, parseInt(text) || 0)
                    }
                }
                Text { text: qsTr("(0-5)"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            }
            Row {
                spacing: Theme.sp1
                Text { text: qsTr("Levels on other sites:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Rectangle {
                    width: 40; height: 24; color: Theme.bg
                    border.color: grabExternalInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: 3
                    TextInput {
                        clip: true
                        id: grabExternalInput
                        anchors { fill: parent; margins: Theme.sp1 }
                        color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                        inputMethodHints: Qt.ImhDigitsOnly
                        text: "0"
                        onTextChanged: grabber.grabExternal = Math.min(5, parseInt(text) || 0)
                    }
                }
            }
        }

        // IDM Site Grabber page filters
        Row {
            spacing: Theme.sp3
            Row {
                spacing: Theme.sp1
                Text { text: qsTr("Explore only:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Rectangle {
                    width: 170; height: 24; color: Theme.bg
                    border.color: grabIncludeInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: 3
                    TextInput {
                        clip: true
                        id: grabIncludeInput
                        anchors { fill: parent; margins: Theme.sp1 }
                        color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                        Text { visible: parent.text.length === 0; text: qsTr("site.com/docs/*"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                        onTextChanged: grabber.grabInclude = text
                    }
                }
            }
            Row {
                spacing: Theme.sp1
                Text { text: qsTr("Never open:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Rectangle {
                    width: 170; height: 24; color: Theme.bg
                    border.color: grabExcludeInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: 3
                    TextInput {
                        clip: true
                        id: grabExcludeInput
                        anchors { fill: parent; margins: Theme.sp1 }
                        color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                        Text { visible: parent.text.length === 0; text: qsTr("*/logout*"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                        onTextChanged: grabber.grabExclude = text
                    }
                }
            }
            Row {
                spacing: Theme.sp1
                Rectangle { width: 16; height: 16; radius: 3; anchors.verticalCenter: parent.verticalCenter
                    color: grabber.grabStayBelow ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                    Text { anchors.centerIn: parent; text: grabber.grabStayBelow ? "✓" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea { anchors.fill: parent; onClicked: grabber.grabStayBelow = !grabber.grabStayBelow }
                }
                Text { text: qsTr("Don't explore parent directories"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; anchors.verticalCenter: parent.verticalCenter }
            }
        }

        // Saved projects and templates (IDM's Recent projects / Project template)
        Row {
            spacing: Theme.sp2
            Text { text: qsTr("Project:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; anchors.verticalCenter: parent.verticalCenter }
            Rectangle {
                width: 170; height: 24; color: Theme.bg
                border.color: grabProjectName.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: 3
                TextInput {
                    clip: true
                    id: grabProjectName
                    anchors { fill: parent; margins: Theme.sp1 }
                    color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                    Text { visible: parent.text.length === 0; text: qsTr("project name"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                }
            }
            Rectangle {
                width: 90; height: 24; radius: 3; color: saveProjArea.containsMouse ? Theme.accent : Theme.accentDark
                Text { anchors.centerIn: parent; text: qsTr("Save project"); color: "white"; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { id: saveProjArea; anchors.fill: parent; hoverEnabled: true; onClicked: grabber.onProjectSave(grabProjectName.text, false) }
            }
            Rectangle {
                width: 100; height: 24; radius: 3; color: saveTplArea.containsMouse ? Theme.accent : Theme.accentDark
                Text { anchors.centerIn: parent; text: qsTr("Save as template"); color: "white"; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { id: saveTplArea; anchors.fill: parent; hoverEnabled: true; onClicked: grabber.onProjectSave(grabProjectName.text, true) }
            }
            Rectangle {
                width: 100; height: 24; radius: 3; color: updProjArea.containsMouse ? Theme.accent : Theme.accentDark
                Text { anchors.centerIn: parent; text: qsTr("Update project"); color: "white"; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { id: updProjArea; anchors.fill: parent; hoverEnabled: true; onClicked: grabber.onProjectUpdate() }
            }
            Row {
                spacing: Theme.sp1
                Rectangle { width: 16; height: 16; radius: 3; anchors.verticalCenter: parent.verticalCenter
                    color: grabber.grabDownloadAll ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                    Text { anchors.centerIn: parent; text: grabber.grabDownloadAll ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea { anchors.fill: parent; onClicked: grabber.grabDownloadAll = !grabber.grabDownloadAll }
                }
                Text { text: qsTr("Download everything found"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; anchors.verticalCenter: parent.verticalCenter }
            }
        }
        Flow {
            width: parent.width; spacing: Theme.sp1
            visible: grabber.projects.length > 0
            Repeater {
                model: grabber.projects
                Rectangle {
                    height: 22; radius: 3; color: Theme.surface; border.color: Theme.surfaceAlt
                    width: projLabel.implicitWidth + 34
                    Text {
                        id: projLabel
                        anchors { left: parent.left; leftMargin: Theme.sp2; verticalCenter: parent.verticalCenter }
                        text: (modelData.is_template ? "[template] " : "") + modelData.name + (modelData.is_template ? "" : " (" + modelData.result_count + ")")
                        color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                    }
                    MouseArea { anchors { left: parent.left; top: parent.top; bottom: parent.bottom; right: closeProj.left } onClicked: { grabProjectName.text = modelData.name; grabber.onProjectOpen(modelData.id) } }
                    Text { id: closeProj; anchors { right: parent.right; rightMargin: Theme.sp1; verticalCenter: parent.verticalCenter } text: "\u2715"; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
                        MouseArea { anchors.fill: parent; anchors.margins: -4; onClicked: grabber.onProjectDelete(modelData.id) } }
                }
            }
        }

        // Results header
        Row {
            width: parent.width; spacing: 0
            Text { width: 30; text: qsTr("#"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            Text { width: 24; text: qsTr(""); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            Text { width: parent.width - 54; text: "URL (" + grabber.grabResults.length + " found)"; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
        }

        Rectangle { width: parent.width; height: 1; color: Theme.surfaceAlt }

        // Results list
        ListView {
            width: parent.width
            // Leave room for the "Add Selected" button below (it used to be pushed
            // under the status bar).
            height: Math.max(80, grabber.height - 260)
            clip: true
            cacheBuffer: 200
            model: grabber.grabResults
            delegate: Rectangle {
                // `ListView.view` only exists on the delegate root; nested items must use this.
                property var lv: ListView.view
                width: lv.width
                height: 22
                color: grabber.grabSelected.indexOf(index) >= 0 ? Theme.surfaceAlt : "transparent"
                Row {
                    anchors.fill: parent
                    Text { width: 30; text: (index + 1); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    Rectangle {
                        width: 14; height: 14; radius: 2
                        color: grabber.grabSelected.indexOf(index) >= 0 ? Theme.accent : Theme.bg
                        border.color: Theme.surfaceAlt; border.width: 1
                        MouseArea { anchors.fill: parent; onClicked: grabber.onToggleResult(index) }
                    }
                    Text { width: lv.width - 54; text: modelData; color: Theme.text; font.pixelSize: Theme.fontSizeSmall; elide: Text.ElideRight }
                }
            }

            Text {
                visible: grabber.grabResults.length === 0 && !grabber.grabLoading
                anchors.centerIn: parent
                text: qsTr("Enter a URL and click Grab")
                color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal
            }
        }

        // Add selected button
        Rectangle {
            width: 140; height: 26; radius: 3
            color: grabber.grabSelected.length > 0 ? Theme.accent : Theme.surfaceAlt
            opacity: grabber.grabSelected.length > 0 ? 1.0 : 0.4
            Text { anchors.centerIn: parent; text: "Add Selected (" + grabber.grabSelected.length + ")"; color: "white"; font.pixelSize: Theme.fontSizeSmall }
            MouseArea { anchors.fill: parent; enabled: grabber.grabSelected.length > 0; onClicked: grabber.onAddSelected() }
        }
    }
}
