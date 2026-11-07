import QtQuick

// FQT-036: Torrent Policy dialog \u2014 extracted from main.qml
AppDialog {
    id: torrentPolicyDialog

    property var policyData: null
    property var togglePolicyField
    property var doPrivacyCheck
    property var saveTorrentPolicy

    title: qsTr("Torrent Policy")

    contentItem: [
        Flickable {
            width: parent.width; height: parent.height - 10
            clip: true; contentHeight: policyContent.height
            Column { id: policyContent; width: parent.width; spacing: Theme.sp3

                Text { text: qsTr("Discovery"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                Row {
                    spacing: Theme.sp2; height: 22
                    Rectangle {
                        width: 16; height: 16; radius: 3
                        property bool checked: torrentPolicyDialog.policyData ? !!torrentPolicyDialog.policyData.dht_enabled : false
                        color: checked ? Theme.accent : Theme.bg
                        border.color: Theme.surfaceAlt; border.width: 1
                        Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                        MouseArea { anchors.fill: parent; onClicked: togglePolicyField("dht_enabled") }
                    }
                    Text { text: qsTr("DHT (distributed hash table)"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                }

                Row {
                    spacing: Theme.sp2; height: 22
                    Rectangle {
                        width: 16; height: 16; radius: 3
                        property bool checked: torrentPolicyDialog.policyData ? !!torrentPolicyDialog.policyData.pex_enabled : false
                        color: checked ? Theme.accent : Theme.bg
                        border.color: Theme.surfaceAlt; border.width: 1
                        Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                        MouseArea { anchors.fill: parent; onClicked: togglePolicyField("pex_enabled") }
                    }
                    Text { text: qsTr("PEX (peer exchange)"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                }

                Row {
                    spacing: Theme.sp2; height: 22
                    Rectangle {
                        width: 16; height: 16; radius: 3
                        property bool checked: torrentPolicyDialog.policyData ? !!torrentPolicyDialog.policyData.lsd_enabled : false
                        color: checked ? Theme.accent : Theme.bg
                        border.color: Theme.surfaceAlt; border.width: 1
                        Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                        MouseArea { anchors.fill: parent; onClicked: togglePolicyField("lsd_enabled") }
                    }
                    Text { text: qsTr("LSD (local service discovery)"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                }

                Text { text: qsTr("Privacy"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                Row {
                    spacing: Theme.sp2; height: 22
                    Rectangle {
                        width: 16; height: 16; radius: 3
                        property bool checked: torrentPolicyDialog.policyData ? !!torrentPolicyDialog.policyData.proxy_required : false
                        color: checked ? Theme.accent : Theme.bg
                        border.color: Theme.surfaceAlt; border.width: 1
                        Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                        MouseArea { anchors.fill: parent; onClicked: togglePolicyField("proxy_required") }
                    }
                    Text { text: qsTr("Require proxy"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                }

                Row {
                    spacing: Theme.sp2; height: 22
                    Rectangle {
                        width: 16; height: 16; radius: 3
                        property bool checked: torrentPolicyDialog.policyData ? !!torrentPolicyDialog.policyData.blocklist_enabled : false
                        color: checked ? Theme.accent : Theme.bg
                        border.color: Theme.surfaceAlt; border.width: 1
                        Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                        MouseArea { anchors.fill: parent; onClicked: togglePolicyField("blocklist_enabled") }
                    }
                    Text { text: qsTr("Enable blocklist"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                }

                Text { text: qsTr("Seeding"); color: Theme.accent; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

                Row {
                    spacing: Theme.sp2; height: 22
                    Rectangle {
                        width: 16; height: 16; radius: 3
                        property bool checked: torrentPolicyDialog.policyData ? !!torrentPolicyDialog.policyData.seed_enabled : false
                        color: checked ? Theme.accent : Theme.bg
                        border.color: Theme.surfaceAlt; border.width: 1
                        Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                        MouseArea { anchors.fill: parent; onClicked: togglePolicyField("seed_enabled") }
                    }
                    Text { text: qsTr("Seed after completion"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                }
            }
        }
    ]

    buttons: [
        Rectangle {
            width: Theme.btnS; height: Theme.controlH; radius: Theme.radius
            color: policyCheckArea.containsMouse ? Theme.surfaceAlt : Theme.surface
            Text { anchors.centerIn: parent; text: qsTr("Check"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            MouseArea { id: policyCheckArea; anchors.fill: parent; hoverEnabled: true; onClicked: if (doPrivacyCheck) doPrivacyCheck() }
        },
        Rectangle {
            width: Theme.btnS; height: Theme.controlH; radius: Theme.radius
            color: policySaveArea.containsMouse ? Theme.accent : Theme.accentDark
            Text { anchors.centerIn: parent; text: qsTr("Save"); color: "white"; font.pixelSize: Theme.fontSizeSmall }
            MouseArea { id: policySaveArea; anchors.fill: parent; hoverEnabled: true; onClicked: if (saveTorrentPolicy) saveTorrentPolicy() }
        },
        Rectangle {
            width: Theme.btnM; height: Theme.controlH; radius: Theme.radius
            color: policyCancelArea.containsMouse ? Theme.border : Theme.surfaceAlt
            Text { anchors.centerIn: parent; text: qsTr("Cancel"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
            MouseArea { id: policyCancelArea; anchors.fill: parent; hoverEnabled: true; onClicked: torrentPolicyDialog.close() }
        }
    ]
}
