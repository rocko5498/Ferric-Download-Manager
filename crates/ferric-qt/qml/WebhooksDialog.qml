import QtQuick

// FQT-085: Webhooks dialog \u2014 extracted from main.qml
AppDialog {
    id: webhooksDialog

    property var bus
    property var requestBus
    property var busError
    property var webhookList: []
    property var loadWebhooks
    property var showError
    property var showInfo

    title: qsTr("Webhooks")

    contentItem: [
        Row { spacing: Theme.sp2; width: parent.width; height: Theme.controlH
            Rectangle {
                width: 70; height: Theme.controlH; radius: Theme.radius
                color: addWebhookArea.containsMouse ? Theme.border : Theme.surfaceAlt
                Text { anchors.centerIn: parent; text: qsTr("+ Add"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { id: addWebhookArea; anchors.fill: parent; hoverEnabled: true
                    onClicked: { webhookUrlInput.text = ""; webhookEventsInput.text = "*"; webhookAddRow.visible = true; }
                }
            }
            Rectangle {
                width: 50; height: Theme.controlH; radius: Theme.radius
                color: whRefreshArea.containsMouse ? Theme.border : Theme.surfaceAlt
                Text { anchors.centerIn: parent; text: qsTr("Refresh"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { id: whRefreshArea; anchors.fill: parent; hoverEnabled: true
                    onClicked: { if (loadWebhooks) loadWebhooks(); }
                }
            }
        },

        // Add webhook row (hidden by default)
        Row { id: webhookAddRow; visible: false; spacing: Theme.sp1; width: parent.width; height: Theme.controlH
            Rectangle { anchors.verticalCenter: parent.verticalCenter; width: parent.width - 140; height: Theme.controlH; color: Theme.bg; border.color: webhookUrlInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                TextInput { id: webhookUrlInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeSmall
                    Text { visible: parent.text.length === 0; text: qsTr("https://..."); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall } } }
            TextInput { id: webhookEventsInput; visible: false; width: 0; height: 0; text: "*" }
            clip: true
            Rectangle { width: 50; height: Theme.controlH; radius: Theme.radius; color: whSaveArea.containsMouse ? Theme.border : Theme.surfaceAlt
                Text { anchors.centerIn: parent; text: qsTr("Save"); color: Theme.success; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { id: whSaveArea; anchors.fill: parent; hoverEnabled: true
                    onClicked: {
                        requestBus("add_webhook", { url: webhookUrlInput.text, events: webhookEventsInput.text }, function(raw) {
                            var err = busError(raw);
                            if (err.length > 0) {
                                if (showError) showError(err);
                            } else {
                                webhookAddRow.visible = false;
                                if (loadWebhooks) loadWebhooks();
                            }
                        });
                    }
                }
            }
            Rectangle { width: 50; height: Theme.controlH; radius: Theme.radius; color: Theme.surfaceAlt
                Text { anchors.centerIn: parent; text: qsTr("Cancel"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { anchors.fill: parent; onClicked: webhookAddRow.visible = false }
            }
        },

        Flickable { width: parent.width; height: parent.height - 60; clip: true; contentHeight: whCol.height
            Column { id: whCol; width: parent.width; spacing: Theme.sp1
                Repeater {
                    model: webhooksDialog.webhookList
                    Row { spacing: Theme.sp1; height: Theme.controlH; width: whCol.width
                        Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 8; height: 8; radius: 4; color: modelData.enabled ? Theme.success : Theme.danger }
                        Column { width: parent.width - 80
                            Text { text: modelData.url; color: Theme.text; font.pixelSize: Theme.fontSizeSmall; elide: Text.ElideRight; width: parent.width }
                            Text { text: "Events: " + modelData.events; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                        }
                    }
                }
            }
        }
    ]

    buttons: [
        Rectangle { width: Theme.btnM; height: Theme.controlH; radius: Theme.radius; color: whCloseArea.containsMouse ? Theme.border : Theme.surfaceAlt
            Text { anchors.centerIn: parent; text: qsTr("Close"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
            MouseArea { id: whCloseArea; anchors.fill: parent; hoverEnabled: true; onClicked: webhooksDialog.close() }
        }
    ]
}
