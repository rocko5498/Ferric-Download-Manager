import QtQuick

// FQT-082: Proxy add sub-dialog \u2014 extracted from main.qml
AppDialog {
    id: proxyAddDialog

    property var bus
    property var requestBus
    property var busError
    property var loadProxies
    property var showError
    property string proxyNewType: "socks5"

    title: qsTr("Add Proxy")

    function openEmpty() {
        proxyNameInput.text = "";
        proxyHostInput.text = "";
        proxyPortInput.text = "1080";
        proxyUserInput.text = "";
        proxyPassInput.text = "";
        proxyNewType = "socks5";
        proxyTypeRepeater.model = ["socks5", "http"];
        open();
    }

    contentItem: [
        Column { width: parent.width; spacing: Theme.sp1
            Text { text: qsTr("Name"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            Rectangle { width: parent.width; height: Theme.controlH; color: Theme.bg; border.color: proxyNameInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                TextInput { id: proxyNameInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal } }
        clip: true
        },
        Column { width: parent.width; spacing: Theme.sp1
            Text { text: qsTr("Type"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            Row { spacing: Theme.sp1
                Repeater { id: proxyTypeRepeater; model: ["socks5", "http"]
                    Rectangle { width: 60; height: Theme.rowH; radius: Theme.radius
                        color: proxyAddDialog.proxyNewType === modelData ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                        Text { anchors.centerIn: parent; text: modelData; color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                        MouseArea { anchors.fill: parent; onClicked: proxyAddDialog.proxyNewType = modelData }
                    }
                }
            }
        },
        Row { spacing: Theme.sp2; width: parent.width
            Column { width: parent.width * 0.7; spacing: Theme.sp1
                Text { text: qsTr("Host"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Rectangle { width: parent.width; height: Theme.controlH; color: Theme.bg; border.color: proxyHostInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                    TextInput { id: proxyHostInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal } }
            clip: true
            }
            Column { width: parent.width * 0.3 - 8; spacing: Theme.sp1
                Text { text: qsTr("Port"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Rectangle { width: parent.width; height: Theme.controlH; color: Theme.bg; border.color: proxyPortInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                    TextInput { id: proxyPortInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal; inputMethodHints: Qt.ImhFormattedNumbersOnly; text: qsTr("1080") } }
            clip: true
            }
        },
        Row { spacing: Theme.sp2; width: parent.width
            Column { width: parent.width * 0.5 - 4; spacing: Theme.sp1
                Text { text: qsTr("Username (optional)"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Rectangle { width: parent.width; height: Theme.controlH; color: Theme.bg; border.color: proxyUserInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                    TextInput { id: proxyUserInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal } }
            clip: true
            }
            Column { width: parent.width * 0.5 - 4; spacing: Theme.sp1
                Text { text: qsTr("Password (optional)"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Rectangle { width: parent.width; height: Theme.controlH; color: Theme.bg; border.color: proxyPassInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: Theme.radius
                    TextInput { id: proxyPassInput; anchors { fill: parent; margins: Theme.sp1 } color: Theme.text; font.pixelSize: Theme.fontSizeNormal; echoMode: TextInput.Password } }
            clip: true
            }
        }
    ]

    buttons: [
        Rectangle { width: Theme.btnM; height: Theme.controlH; radius: Theme.radius; color: proxySaveArea.containsMouse ? Theme.border : Theme.surfaceAlt
            Text { anchors.centerIn: parent; text: qsTr("Save"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
            MouseArea { id: proxySaveArea; anchors.fill: parent; hoverEnabled: true
                onClicked: {
                    var json = JSON.stringify({
                        name: proxyNameInput.text,
                        proxy_type: proxyAddDialog.proxyNewType,
                        host: proxyHostInput.text,
                        port: parseInt(proxyPortInput.text) || 1080,
                        username: proxyUserInput.text || null,
                        password: proxyPassInput.text || null
                    });
                    requestBus("add_proxy", { json: json }, function(raw) {
                        var err = busError(raw);
                        if (err.length > 0) {
                            if (showError) showError(err);
                        } else {
                            proxyAddDialog.close();
                            if (loadProxies) loadProxies();
                        }
                    });
                }
            }
        },
        Rectangle { width: Theme.btnM; height: Theme.controlH; radius: Theme.radius; color: proxyCancelArea.containsMouse ? Theme.border : Theme.surfaceAlt
            Text { anchors.centerIn: parent; text: qsTr("Cancel"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
            MouseArea { id: proxyCancelArea; anchors.fill: parent; hoverEnabled: true; onClicked: proxyAddDialog.close() }
        }
    ]
}
