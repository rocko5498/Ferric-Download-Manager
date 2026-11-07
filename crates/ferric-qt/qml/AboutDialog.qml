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
        Row { spacing: Theme.sp2; height: Theme.rowH
            Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 8; height: 8; radius: 4; color: session && session.connected ? Theme.success : Theme.danger }
            Text { anchors.verticalCenter: parent.verticalCenter; color: session && session.connected ? Theme.success : Theme.danger; font.pixelSize: Theme.fontSizeSmall
                text: session && session.connected ? "Daemon connected (port " + session.port + ")" : "Daemon disconnected" }
        },

        Column { width: parent.width; spacing: Theme.sp1
            Text { text: qsTr("Version") + " " + Qt.application.version; color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
            Text { text: "Qt 6.8.3"; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
        },

        Column { width: parent.width; spacing: Theme.sp1
            Text { text: qsTr("Open folders"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }
            Row { spacing: Theme.sp1
                Rectangle { width: 90; height: Theme.rowH; radius: Theme.radius; color: aboutDataArea.containsMouse ? Theme.border : Theme.surfaceAlt
                    Text { anchors.centerIn: parent; text: qsTr("Data folder"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
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

        Column { width: parent.width; spacing: Theme.sp1
            Text { text: qsTr("Licenses"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }
            Text { text: qsTr("Ferric is licensed under the MIT License. Qt is used under LGPLv3. See LICENSE file for details."); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; wrapMode: Text.Wrap; width: parent.width }
            Text { text: qsTr("This product uses Qt, which is licensed under the GNU Lesser General Public License v3."); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; wrapMode: Text.Wrap; width: parent.width }
        }
    ]

    buttons: [
        Rectangle { width: Theme.btnM; height: Theme.controlH; radius: Theme.radius; color: aboutCloseArea.containsMouse ? Theme.border : Theme.surfaceAlt
            Text { anchors.centerIn: parent; text: qsTr("Close"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
            MouseArea { id: aboutCloseArea; anchors.fill: parent; hoverEnabled: true; onClicked: aboutDialog.close() }
        }
    ]
}
