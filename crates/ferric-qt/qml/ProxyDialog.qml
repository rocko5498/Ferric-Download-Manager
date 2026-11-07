import QtQuick

// FQT-082: Proxy Manager dialog \u2014 extracted from main.qml
AppDialog {
    id: proxyDialog

    property var bus
    property var requestBus
    property var proxyList: []
    property var loadProxies
    property var showError
    property var showInfo
    property var openAdd

    title: qsTr("Proxy Manager")

    contentItem: [
        Row { spacing: 8; width: parent.width; height: 28
            Rectangle {
                width: 70; height: 24; radius: 3
                color: addProxyArea.containsMouse ? Theme.border : Theme.surfaceAlt
                Text { anchors.centerIn: parent; text: qsTr("+ Add"); color: Theme.text; font.pixelSize: 11 }
                MouseArea { id: addProxyArea; anchors.fill: parent; hoverEnabled: true
                    onClicked: { if (openAdd) openAdd(); }
                }
            }
            Rectangle {
                width: 50; height: 24; radius: 3
                color: proxyRefreshArea.containsMouse ? Theme.border : Theme.surfaceAlt
                Text { anchors.centerIn: parent; text: qsTr("Refresh"); color: Theme.text; font.pixelSize: 11 }
                MouseArea { id: proxyRefreshArea; anchors.fill: parent; hoverEnabled: true
                    onClicked: { if (loadProxies) loadProxies(); }
                }
            }
        },

        Flickable { width: parent.width; height: parent.height - 40; clip: true; contentHeight: proxyCol.height
            Column { id: proxyCol; width: parent.width; spacing: 4
                Repeater {
                    model: proxyDialog.proxyList
                    Row { spacing: 6; height: 28; width: proxyCol.width
                        Rectangle { width: 8; height: 8; radius: 4; color: Theme.accent }
                        Column { width: parent.width - 130
                            Text { text: modelData.name + " (" + modelData.proxy_type + ")"; color: Theme.text; font.pixelSize: 11 }
                            Text { text: modelData.host + ":" + modelData.port; color: Theme.textDim; font.pixelSize: 10 }
                        }
                        Rectangle {
                            width: 45; height: 20; radius: 3
                            color: proxyTestBtn.containsMouse ? Theme.border : Theme.surfaceAlt
                            Text { anchors.centerIn: parent; text: qsTr("Test"); color: Theme.success; font.pixelSize: 10 }
                            MouseArea { id: proxyTestBtn; anchors.fill: parent; hoverEnabled: true
                                onClicked: {
                                    requestBus("test_proxy", { id: modelData.id }, function(result) {
                                        try {
                                            var r = JSON.parse(result);
                                            if (r.ok) {
                                                if (showInfo) showInfo("Proxy OK \u2014 " + r.latency_ms + "ms");
                                            } else {
                                                if (showError) showError("Proxy failed: " + (r.error || "unknown"));
                                            }
                                        } catch(e) {
                                            if (showError) showError("Test failed: " + e);
                                        }
                                    });
                                }
                            }
                        }
                        Rectangle {
                            width: 45; height: 20; radius: 3
                            color: proxyDelBtn.containsMouse ? Theme.border : Theme.surfaceAlt
                            Text { anchors.centerIn: parent; text: qsTr("Delete"); color: Theme.danger; font.pixelSize: 10 }
                            MouseArea { id: proxyDelBtn; anchors.fill: parent; hoverEnabled: true
                                onClicked: {
                                    requestBus("delete_proxy", { id: modelData.id }, function() {
                                        if (loadProxies) loadProxies();
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    ]

    buttons: [
        Rectangle { width: 70; height: 26; radius: 3; color: proxyCloseArea.containsMouse ? Theme.border : Theme.surfaceAlt
            Text { anchors.centerIn: parent; text: qsTr("Close"); color: Theme.text; font.pixelSize: 12 }
            MouseArea { id: proxyCloseArea; anchors.fill: parent; hoverEnabled: true; onClicked: proxyDialog.close() }
        }
    ]
}
