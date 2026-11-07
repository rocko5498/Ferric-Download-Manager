import QtQuick

// Exit-while-active dialog \u2014 extracted from main.qml (Phase 2 split #6)
AppDialog {
    id: exitDialog

    property var onMinimize: function() {}
    property var onExitKeepRunning: function() {}
    property var onExitPauseAll: function() {}

    title: qsTr("Downloads in progress")

    contentItem: [
        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            text: qsTr("What would you like to do?")
            color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
        }
    ]

    buttons: [
        Rectangle {
            width: Theme.btnL; height: Theme.controlH; radius: Theme.radius; color: Theme.surfaceAlt
            Text { anchors.centerIn: parent; text: qsTr("Minimize to tray"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
            MouseArea {
                anchors.fill: parent
                onClicked: { exitDialog.onMinimize(); exitDialog.close(); }
            }
        },
        Rectangle {
            width: Theme.btnL; height: Theme.controlH; radius: Theme.radius; color: Theme.surfaceAlt
            Text { anchors.centerIn: parent; text: qsTr("Exit, keep running"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
            MouseArea { anchors.fill: parent; onClicked: { exitDialog.onExitKeepRunning(); exitDialog.close(); } }
        },
        Rectangle {
            width: Theme.btnL; height: Theme.controlH; radius: Theme.radius; color: Theme.danger
            Text { anchors.centerIn: parent; text: qsTr("Pause all, exit"); color: "#ffffff"; font.pixelSize: Theme.fontSizeSmall }
            MouseArea {
                anchors.fill: parent
                onClicked: { exitDialog.onExitPauseAll(); exitDialog.close(); }
            }
        },
        Rectangle {
            width: Theme.btnL; height: Theme.controlH; radius: Theme.radius; color: Theme.bg
            Text { anchors.centerIn: parent; text: qsTr("Cancel"); color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal }
            MouseArea { anchors.fill: parent; onClicked: exitDialog.close() }
        }
    ]
}
