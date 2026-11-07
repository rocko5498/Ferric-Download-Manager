import QtQuick

// Kill-switch persistent banner \u2014 extracted from main.qml (T3 differentiator)
Rectangle {
    id: killSwitchBanner

    visible: active
    height: active ? 32 : 0
    color: Theme.danger
    border.color: Theme.danger
    border.width: 1

    property bool active: false
    property var onResume: function() {}
    property var onDismiss: function() {}

    Row {
        anchors.centerIn: parent
        spacing: 12

        // Pulsing dot
        Rectangle {
            width: 8; height: 8; radius: 4
            color: Theme.danger
            anchors.verticalCenter: parent.verticalCenter
            SequentialAnimation on opacity {
                loops: Animation.Infinite
                NumberAnimation { to: 0.3; duration: 800 }
                NumberAnimation { to: 1.0; duration: 800 }
            }
        }

        Text {
            text: qsTr("Kill switch ACTIVE \u2014 torrents paused, proxy unreachable")
            color: Theme.danger; font.pixelSize: 12; font.bold: true
            anchors.verticalCenter: parent.verticalCenter
        }

        Rectangle {
            width: 120; height: 22; radius: 3
            color: resumeBtnArea.containsMouse ? Theme.accent : Theme.danger
            Text { anchors.centerIn: parent; text: qsTr("Wait for proxy"); color: Theme.danger; font.pixelSize: 10 }
            MouseArea {
                id: resumeBtnArea
                anchors.fill: parent
                hoverEnabled: true
                onClicked: killSwitchBanner.onResume()
            }
        }

        Rectangle {
            width: 100; height: 22; radius: 3
            color: dismissBtnArea.containsMouse ? Theme.border : Theme.surfaceAlt
            Text { anchors.centerIn: parent; text: qsTr("Continue exposed"); color: Theme.textDim; font.pixelSize: 10 }
            MouseArea {
                id: dismissBtnArea
                anchors.fill: parent
                hoverEnabled: true
                onClicked: killSwitchBanner.onDismiss()
            }
        }
    }
}
