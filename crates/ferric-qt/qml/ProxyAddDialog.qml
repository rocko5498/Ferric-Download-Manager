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
        Column { width: parent.width; spacing: 3
            Text { text: qsTr("Name"); color: Theme.textDim; font.pixelSize: 11 }
            Rectangle { width: parent.width; height: 26; color: Theme.bg; border.color: proxyNameInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: 3
                TextInput { id: proxyNameInput; anchors { fill: parent; margins: 4 } color: Theme.text; font.pixelSize: 12 } }
        clip: true
        },
        Column { width: parent.width; spacing: 3
            Text { text: qsTr("Type"); color: Theme.textDim; font.pixelSize: 11 }
            Row { spacing: 6
                Repeater { id: proxyTypeRepeater; model: ["socks5", "http"]
                    Rectangle { width: 60; height: 22; radius: 3
                        color: proxyAddDialog.proxyNewType === modelData ? Theme.accent : Theme.bg; border.color: Theme.surfaceAlt; border.width: 1
                        Text { anchors.centerIn: parent; text: modelData; color: Theme.text; font.pixelSize: 11 }
                        MouseArea { anchors.fill: parent; onClicked: proxyAddDialog.proxyNewType = modelData }
                    }
                }
            }
        },
        Row { spacing: 8; width: parent.width
            Column { width: parent.width * 0.7; spacing: 3
                Text { text: qsTr("Host"); color: Theme.textDim; font.pixelSize: 11 }
                Rectangle { width: parent.width; height: 26; color: Theme.bg; border.color: proxyHostInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: 3
                    TextInput { id: proxyHostInput; anchors { fill: parent; margins: 4 } color: Theme.text; font.pixelSize: 12 } }
            clip: true
            }
            Column { width: parent.width * 0.3 - 8; spacing: 3
                Text { text: qsTr("Port"); color: Theme.textDim; font.pixelSize: 11 }
                Rectangle { width: parent.width; height: 26; color: Theme.bg; border.color: proxyPortInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: 3
                    TextInput { id: proxyPortInput; anchors { fill: parent; margins: 4 } color: Theme.text; font.pixelSize: 12; inputMethodHints: Qt.ImhFormattedNumbersOnly; text: qsTr("1080") } }
            clip: true
            }
        },
        Row { spacing: 8; width: parent.width
            Column { width: parent.width * 0.5 - 4; spacing: 3
                Text { text: qsTr("Username (optional)"); color: Theme.textDim; font.pixelSize: 11 }
                Rectangle { width: parent.width; height: 26; color: Theme.bg; border.color: proxyUserInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: 3
                    TextInput { id: proxyUserInput; anchors { fill: parent; margins: 4 } color: Theme.text; font.pixelSize: 12 } }
            clip: true
            }
            Column { width: parent.width * 0.5 - 4; spacing: 3
                Text { text: qsTr("Password (optional)"); color: Theme.textDim; font.pixelSize: 11 }
                Rectangle { width: parent.width; height: 26; color: Theme.bg; border.color: proxyPassInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: 3
                    TextInput { id: proxyPassInput; anchors { fill: parent; margins: 4 } color: Theme.text; font.pixelSize: 12; echoMode: TextInput.Password } }
            clip: true
            }
        }
    ]

    buttons: [
        Rectangle { width: 60; height: 26; radius: 3; color: proxySaveArea.containsMouse ? Theme.border : Theme.surfaceAlt
            Text { anchors.centerIn: parent; text: qsTr("Save"); color: Theme.text; font.pixelSize: 12 }
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
        Rectangle { width: 60; height: 26; radius: 3; color: proxyCancelArea.containsMouse ? Theme.border : Theme.surfaceAlt
            Text { anchors.centerIn: parent; text: qsTr("Cancel"); color: Theme.text; font.pixelSize: 12 }
            MouseArea { id: proxyCancelArea; anchors.fill: parent; hoverEnabled: true; onClicked: proxyAddDialog.close() }
        }
    ]
}
