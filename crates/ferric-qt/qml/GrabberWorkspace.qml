import QtQuick
import QtQuick.Controls as QQC

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
        grabUrlField.text = cfg.url || "";
        grabExtField.text = cfg.extensions || "";
        grabDepthField.text = String(cfg.depth || 0);
        grabExternalField.text = String(cfg.external_levels || 0);
        grabIncludeField.text = cfg.include || "";
        grabExcludeField.text = cfg.exclude || "";
        grabber.grabStayBelow = cfg.stay_below === true;
    }
    property var grabResults: []
    property var grabSelected: []
    property bool grabLoading: false

    // Functions
    property var onGrab: function() {}
    property var onToggleResult: function(idx) {}
    property var onAddSelected: function() {}

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
            Text { text: qsTr("Site Grabber"); color: Theme.text; font.pixelSize: Theme.fontSizeLarge; font.weight: Font.Bold }
            Text { text: qsTr("Find and download the files linked from a web page."); color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal }
        }

        // Where to look
        Card {
            Row {
                spacing: Theme.sp2
                LabeledField {
                    id: grabUrlField
                    label: qsTr("Page address")
                    placeholder: qsTr("https://example.com/page")
                    fieldWidth: Math.max(233, parent.parent.width - Theme.btnM - Theme.sp2)
                    // The typed URL must reach the root state (it used to stay in this field, so Grab always
                    // answered "URL required").
                    onTextChanged: grabber.grabUrl = text
                    onAccepted: grabber.onGrab()
                }
                Item {
                    width: grabBtn.width; height: grabUrlField.height
                    PillButton { id: grabBtn; anchors.bottom: parent.bottom; label: grabber.grabLoading ? "..." : qsTr("Grab"); onClicked: grabber.onGrab() }
                }
            }
        }

        // What to look for
        Card {
            heading: qsTr("What to find")
            Flow {
                width: parent.width; spacing: Theme.sp4
                LabeledField { id: grabExtField; label: qsTr("File types"); placeholder: qsTr("mp4, zip, pdf"); onTextChanged: grabber.grabExtensions = text }
                LabeledField { id: grabDepthField; label: qsTr("Link depth (0-5)"); fieldWidth: 89; hints: Qt.ImhDigitsOnly; text: "0"; onTextChanged: grabber.grabDepth = Math.min(5, parseInt(text) || 0) }
                LabeledField { id: grabExternalField; label: qsTr("Levels on other sites"); fieldWidth: 89; hints: Qt.ImhDigitsOnly; text: "0"; onTextChanged: grabber.grabExternal = Math.min(5, parseInt(text) || 0) }
                LabeledField { id: grabIncludeField; label: qsTr("Explore only"); placeholder: qsTr("site.com/docs/*"); onTextChanged: grabber.grabInclude = text }
                LabeledField { id: grabExcludeField; label: qsTr("Never open"); placeholder: qsTr("*/logout*"); onTextChanged: grabber.grabExclude = text }
                Item {
                    width: stayBelow.width; height: Theme.sp1 + 13 + Theme.controlH
                    CheckRow { id: stayBelow; anchors.bottom: parent.bottom; anchors.bottomMargin: (Theme.controlH - Theme.sp4) / 2
                        label: qsTr("Stay below the starting folder"); checked: grabber.grabStayBelow; onToggled: grabber.grabStayBelow = !grabber.grabStayBelow }
                }
            }
        }

        // Saved projects and templates
        Card {
            heading: qsTr("Projects")
            Flow {
                width: parent.width; spacing: Theme.sp3
                LabeledField { id: grabProjectField; label: qsTr("Project name"); placeholder: qsTr("my project") }
                Item {
                    width: projButtons.width; height: Theme.sp1 + 13 + Theme.controlH
                    Row {
                        id: projButtons
                        anchors.bottom: parent.bottom
                        spacing: Theme.sp2
                        PillButton { label: qsTr("Save project"); onClicked: grabber.onProjectSave(grabProjectField.text, false) }
                        PillButton { label: qsTr("Save as template"); primary: false; onClicked: grabber.onProjectSave(grabProjectField.text, true) }
                        PillButton { label: qsTr("Update project"); primary: false; onClicked: grabber.onProjectUpdate() }
                    }
                }
                Item {
                    width: allRow.width; height: Theme.sp1 + 13 + Theme.controlH
                    CheckRow { id: allRow; anchors.bottom: parent.bottom; anchors.bottomMargin: (Theme.controlH - Theme.sp4) / 2
                        label: qsTr("Download everything found"); checked: grabber.grabDownloadAll; onToggled: grabber.grabDownloadAll = !grabber.grabDownloadAll }
                }
            }
        }

        Flow {
            width: parent.width; spacing: Theme.sp1
            visible: grabber.projects.length > 0
            Repeater {
                model: grabber.projects
                Rectangle {
                    height: Theme.rowH; radius: Theme.radius; color: Theme.surface; border.color: Theme.surfaceAlt
                    width: projLabel.implicitWidth + 34
                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        id: projLabel
                        anchors { left: parent.left; leftMargin: Theme.sp2; verticalCenter: parent.verticalCenter }
                        text: (modelData.is_template ? "[template] " : "") + modelData.name + (modelData.is_template ? "" : " (" + modelData.result_count + ")")
                        color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                    }
                    MouseArea { anchors { left: parent.left; top: parent.top; bottom: parent.bottom; right: closeProj.left } onClicked: { grabProjectField.text = modelData.name; grabber.onProjectOpen(modelData.id) } }
                    Text { id: closeProj; anchors { right: parent.right; rightMargin: Theme.sp1; verticalCenter: parent.verticalCenter } text: "\u2715"; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
                        MouseArea { anchors.fill: parent; anchors.margins: -4; onClicked: grabber.onProjectDelete(modelData.id) } }
                }
            }
        }

        // Results header
        Row {
            width: parent.width; spacing: 0
            Text { anchors.verticalCenter: parent.verticalCenter; width: 30; text: qsTr("#"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            Text { anchors.verticalCenter: parent.verticalCenter; width: 24; text: qsTr(""); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            Text { anchors.verticalCenter: parent.verticalCenter; width: parent.width - 54; text: "URL (" + grabber.grabResults.length + " found)"; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
        }

        Rectangle { width: parent.width; height: 1; color: Theme.surfaceAlt }

        // Results list
        ListView {
            width: parent.width
            // Leave room for the "Add Selected" button below (it used to be pushed
            // under the status bar).
            height: Math.max(200, Math.round(grabber.height * 0.45))
            clip: true
            cacheBuffer: 200
            model: grabber.grabResults
            delegate: Rectangle {
                // `ListView.view` only exists on the delegate root; nested items must use this.
                property var lv: ListView.view
                width: lv.width
                height: Theme.rowH
                color: grabber.grabSelected.indexOf(index) >= 0 ? Theme.surfaceAlt : "transparent"
                Row {
                    anchors.fill: parent
                    Text { anchors.verticalCenter: parent.verticalCenter; width: 30; text: (index + 1); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    Rectangle {
                        width: Theme.sp4; height: Theme.sp4; radius: Theme.radius
                        color: grabber.grabSelected.indexOf(index) >= 0 ? Theme.accent : Theme.bg
                        border.color: Theme.surfaceAlt; border.width: 1
                        MouseArea { anchors.fill: parent; onClicked: grabber.onToggleResult(index) }
                    }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: lv.width - 54; text: modelData; color: Theme.text; font.pixelSize: Theme.fontSizeSmall; elide: Text.ElideRight }
                }
            }

            Text {
                visible: grabber.grabResults.length === 0 && !grabber.grabLoading
                anchors.centerIn: parent
                text: qsTr("Enter a web page address above and press Grab to list its files.")
                color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal
            }
        }

        // Add selected button
        PillButton {
            label: qsTr("Add selected") + " (" + grabber.grabSelected.length + ")"
            opacity: grabber.grabSelected.length > 0 ? 1.0 : 0.4
            onClicked: if (grabber.grabSelected.length > 0) grabber.onAddSelected()
        }
    }
    }
}
