import QtQuick

// Error dialog (FQT-021) \u2014 extracted from main.qml
AppDialog {
    id: errorDialog

    property string message: ""

    title: ""

    function show(msg) {
        message = msg
        open()
    }

    contentItem: [
        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            text: errorDialog.message
            color: Theme.danger; font.pixelSize: Theme.fontSizeNormal
            wrapMode: Text.WordWrap
            width: 300
            horizontalAlignment: Text.AlignHCenter
        }
    ]

    buttons: [
        Rectangle {
            width: Theme.btnM; height: Theme.controlH; radius: Theme.radius; color: Theme.surfaceAlt
            Text { anchors.centerIn: parent; text: qsTr("OK"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
            MouseArea { anchors.fill: parent; onClicked: errorDialog.close() }
        }
    ]
}
