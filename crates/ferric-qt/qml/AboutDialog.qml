import QtQuick

// FQT-088: About dialog \u2014 extracted from main.qml
AppDialog {
    id: aboutDialog

    property var bus
    property var requestBus
    property var session
    property var showError

    title: qsTr("Ferric Download Manager")

    contentItem: [
        Row { spacing: 8; height: 22
            Rectangle { width: 8; height: 8; radius: 4; color: session && session.connected ? Theme.success : Theme.danger }
            Text { color: session && session.connected ? Theme.success : Theme.danger; font.pixelSize: 11
                text: session && session.connected ? "Daemon connected (port " + session.port + ")" : "Daemon disconnected" }
        },

        Column { width: parent.width; spacing: 4
            Text { text: "ferric-qt v" + (typeof ferricVersion !== "undefined" ? ferricVersion : "dev"); color: Theme.text; font.pixelSize: 11 }
            Text { text: "ferricd v" + (typeof ferricdVersion !== "undefined" ? ferricdVersion : "dev"); color: Theme.text; font.pixelSize: 11 }
            Text { text: "Qt 6.8.3"; color: Theme.textDim; font.pixelSize: 10 }
        },

        Column { width: parent.width; spacing: 4
            Text { text: qsTr("Open folders"); color: Theme.accent; font.pixelSize: 12; font.bold: true }
            Row { spacing: 6
                Rectangle { width: 90; height: 22; radius: 3; color: aboutDataArea.containsMouse ? Theme.border : Theme.surfaceAlt
                    Text { anchors.centerIn: parent; text: qsTr("Data folder"); color: Theme.text; font.pixelSize: 10 }
                    MouseArea { id: aboutDataArea; anchors.fill: parent; hoverEnabled: true
                        onClicked: {
                            requestBus("get_health", {}, function(raw) {
                                try {
                                    var h = JSON.parse(raw);
                                    bus.open_folder(h.data_dir);
                                } catch(e) {
                                    if (showError) showError("Could not resolve data folder");
                                }
                            });
                        }
                    }
                }
            }
        },

        Column { width: parent.width; spacing: 4
            Text { text: qsTr("Licenses"); color: Theme.accent; font.pixelSize: 12; font.bold: true }
            Text { text: qsTr("Ferric is licensed under the MIT License. Qt is used under LGPLv3. See LICENSE file for details."); color: Theme.textDim; font.pixelSize: 10; wrapMode: Text.Wrap; width: parent.width }
            Text { text: qsTr("This product uses Qt, which is licensed under the GNU Lesser General Public License v3."); color: Theme.textDim; font.pixelSize: 10; wrapMode: Text.Wrap; width: parent.width }
        }
    ]

    buttons: [
        Rectangle { width: 70; height: 26; radius: 3; color: aboutCloseArea.containsMouse ? Theme.border : Theme.surfaceAlt
            Text { anchors.centerIn: parent; text: qsTr("Close"); color: Theme.text; font.pixelSize: 12 }
            MouseArea { id: aboutCloseArea; anchors.fill: parent; hoverEnabled: true; onClicked: aboutDialog.close() }
        }
    ]
}
