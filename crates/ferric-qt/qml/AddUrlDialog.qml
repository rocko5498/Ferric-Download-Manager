import QtQuick
import Qt.labs.platform as Platform

// Add URL dialog \u2014 two-step flow: URL \u2192 File Info
AppDialog {
    id: addUrlDialog

    Platform.FolderDialog {
        id: urlSaveFolderDialog
        title: qsTr("Select save folder")
        onAccepted: {
            var p = folder.toString().replace(/^file:\/\/\//, "").replace(/^file:\/\//, "");
            if (p.length >= 3 && p.charAt(0) === "/" && p.charAt(2) === ":")
                p = p.substring(1);
            saveDirField.text = decodeURIComponent(p);
        }
    }

    property var onAdd: function(url, filename, savePath, cookies, description) {}
    property alias urlText: urlField.text
    property alias filenameText: filenameField.text
    property alias saveDirText: saveDirField.text
    property alias cookiesText: cookiesField.text
    property int step: 1
    // IDM: after the address the Download File Info dialog takes over, so step 2 is skipped.
    property bool skipInfoStep: false

    title: ""

    contentItem: [
        // Step indicator (IDM goes straight from the address to the Download File Info dialog)
        Row {
            visible: !addUrlDialog.skipInfoStep
            spacing: 8
            Text {
                text: qsTr("Step") + " 1: " + qsTr("URL")
                color: step === 1 ? Theme.accent : Theme.textDim
                font.pixelSize: 11; font.bold: step === 1
            }
            Text { text: "\u2192"; color: Theme.textDim; font.pixelSize: 11 }
            Text {
                text: qsTr("Step") + " 2: " + qsTr("File Info")
                color: step === 2 ? Theme.accent : Theme.textDim
                font.pixelSize: 11; font.bold: step === 2
            }
        },

        Text { text: step === 1 ? qsTr("Add Download") : qsTr("Download File Info"); color: Theme.text; font.pixelSize: 14 },

        // Step 1: URL + auth fields
        Column {
            visible: step === 1
            width: parent.width; spacing: 10

            FormField {
                id: urlField
                label: qsTr("URL *")
                placeholder: qsTr("https://\u2026")
                fieldWidth: parent.width
                onAccepted: step2()
            }

            FormField {
                id: cookiesField
                label: qsTr("Cookies (optional \u2014 name=val; n2=v2)")
                fieldWidth: parent.width
            }

            FormField {
                id: referrerField
                label: qsTr("Referrer (optional)")
                fieldWidth: parent.width
            }
        },

        // Step 2: File info fields
        Column {
            visible: step === 2
            width: parent.width; spacing: 10

            FormField {
                id: filenameField
                label: qsTr("Filename (optional)")
                fieldWidth: parent.width
            }

            FormField {
                id: descriptionField
                label: qsTr("Description (optional)")
                fieldWidth: parent.width
            }

            Row {
                width: parent.width
                spacing: 6
                FormField {
                    id: saveDirField
                    label: qsTr("Save dir (optional)")
                    placeholder: qsTr("Leave blank for default download directory")
                    fieldWidth: parent.width - 74
                }
                Rectangle {
                    width: 68; height: 28; radius: 3; color: Theme.surfaceAlt
                    anchors.bottom: parent.bottom
                    Text { anchors.centerIn: parent; text: qsTr("Browse…"); color: Theme.text; font.pixelSize: 11 }
                    MouseArea { anchors.fill: parent; onClicked: urlSaveFolderDialog.open() }
                }
            }

            Item {
                width: parent.width; height: 36

                Row {
                    spacing: 6
                    Rectangle {
                        id: startPausedCheck
                        property bool checked: false
                        width: 16; height: 16; radius: 3
                        color: checked ? Theme.accent : Theme.bg
                        border.color: Theme.surfaceAlt; border.width: 1
                        Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: 10 }
                        MouseArea { anchors.fill: parent; onClicked: parent.checked = !parent.checked }
                    }
                    Text { text: qsTr("Start paused"); color: Theme.textDim; font.pixelSize: 12 }
                }
            }
        }
    ]

    buttons: [
        Rectangle {
            width: 68; height: 28; radius: 4; color: Theme.bg
            Text { anchors.centerIn: parent; text: step === 1 ? qsTr("Cancel") : qsTr("Back"); color: Theme.textDim; font.pixelSize: 12 }
            MouseArea {
                anchors.fill: parent
                onClicked: {
                    if (step === 1) {
                        addUrlDialog.close()
                    } else {
                        step = 1
                    }
                }
            }
        },
        Rectangle {
            width: 60; height: 28; radius: 4; color: Theme.accent
            Text { anchors.centerIn: parent; text: step === 1 ? qsTr("Next") : qsTr("Add"); color: "#fff"; font.pixelSize: 12 }
            MouseArea {
                anchors.fill: parent
                onClicked: {
                    if (step === 1) { step2() }
                    else { addUrlDialog.onAdd(urlField.text, filenameField.text, saveDirField.text, cookiesField.text, descriptionField.text) }
                }
            }
        }
    ]

    function step2() {
        if (urlField.text.length === 0) return
        if (skipInfoStep) {
            addUrlDialog.onAdd(urlField.text.trim(), "", "", cookiesField.text, "")
            return
        }
        if (filenameField.text.length === 0) {
            var parts = urlField.text.split("/")
            var last = parts[parts.length - 1].split("?")[0].split("#")[0]
            if (last.length > 0) filenameField.text = decodeURIComponent(last)
        }
        step = 2
    }

    // Reset when it closes, so the next open never shows the previous address (the clipboard path
    // sets its text *before* opening, so clearing on open would wipe it).
    onVisibleChanged: if (!visible) clearInputs()

    function clearInputs() {
        urlField.text = "";
        filenameField.text = "";
        saveDirField.text = "";
        cookiesField.text = "";
        referrerField.text = "";
        descriptionField.text = "";
        startPausedCheck.checked = false;
        step = 1;
    }
}
